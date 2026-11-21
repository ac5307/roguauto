//! Owns the Wayland surface lifecycle, input, and frame presentation.

use smithay_client_toolkit::{
  delegate_registry, delegate_dispatch2, registry_handlers,
  registry::{ProvidesRegistryState, RegistryState},
  compositor::{CompositorHandler, CompositorState, FrameCallbackData},
  output::{OutputHandler, OutputState},
  shell::{
    WaylandSurface,
    wlr_layer::{
      LayerShellHandler, Anchor, KeyboardInteractivity, Layer, LayerShell,
      LayerSurface, LayerSurfaceConfigure,
    },
  },
  seat::{
    SeatHandler, SeatState, Capability,
    keyboard::{KeyboardHandler, KeyEvent, Keysym, Modifiers, RawModifiers},
    pointer::{BTN_LEFT, PointerHandler, PointerEvent, PointerEventKind},
  },
  shm::{ShmHandler, Shm, slot::SlotPool, CreatePoolError},
  reexports::client::{
    Connection, QueueHandle,
    globals::GlobalList,
    protocol::{
      wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
    },
  },
};

use std::{io, mem};

use crate::{
  controller::{
    ipc::DaemonCommand, logind::LogindPowerExecutor, app_event::AppEvent,
    MenuCommand, MenuEffect, MenuController,
  },
  domain::{
    PowerAction,
    frame_size::{BYTES_PER_PIXEL, FrameSize},
    config::menu_config::MenuConfig,
  },
  view::{AllocationError, MenuView},
};

/// Owns the current layer and tracks whether the compositor has configured it.
enum MenuSurfaceState {
  Hidden {
    last_size: FrameSize,
  },
  WaitingForConfigure {
    layer: LayerSurface,
    last_size: FrameSize,
  },
  Visible {
    layer: LayerSurface,
    size: FrameSize,
  },
}

impl MenuSurfaceState {
  /// Returns the current layer, including while its first configure is pending.
  fn layer(&self) -> Option<&LayerSurface> {
    match self {
      Self::Hidden { .. } => None,
      Self::WaitingForConfigure { layer, .. }
      | Self::Visible { layer, .. } => Some(layer),
    }
  }

  /// Keeps hit testing and reload comparisons based on the last known size.
  fn size(&self) -> FrameSize {
    match self {
      Self::Hidden { last_size }
      | Self::WaitingForConfigure { last_size, .. } => *last_size,
      Self::Visible { size, .. } => *size,
    }
  }

  /// Records the configured size and keeps ownership of the current layer.
  fn configure(&mut self, size: FrameSize) {
    let last_size = self.size();
    let previous = mem::replace(self, Self::Hidden { last_size });
    *self = match previous {
      Self::WaitingForConfigure { layer, .. }
      | Self::Visible { layer, .. } => Self::Visible { layer, size },
      hidden @ Self::Hidden { .. } => hidden,
    };
  }
}

/// Tracks keyboard focus on the current layer.
#[derive(PartialEq, Eq)]
enum KeyboardFocus {
  Unfocused,
  Focused,
}

/// Tracks whether Calloop should dispatch another batch of events.
#[derive(PartialEq, Eq)]
enum DaemonState {
  Running,
  Stopping,
}

/// Owns the Wayland resources and menu state on the main event loop.
pub struct WaylandHost {
  /*
   * Core Wayland state.
   */
  registry_state: RegistryState,
  compositor_state: CompositorState,
  output_state: OutputState,
  seat_state: SeatState,
  shm_state: Shm,

  /*
   * Layer-shell objects.
   */
  layer_shell: LayerShell,

  /*
   * Input objects.
   */
  keyboard: Option<wl_keyboard::WlKeyboard>,
  pointer: Option<wl_pointer::WlPointer>,
  keyboard_focus: KeyboardFocus,

  /*
   * Shared-memory rendering.
   */
  pool: SlotPool,
  view: MenuView,

  /*
   * Application services.
   */
  power_executor: LogindPowerExecutor,
  controller: MenuController,

  /*
   * Daemon/UI state.
   */
  daemon_state: DaemonState,
  surface_state: MenuSurfaceState,
}

impl WaylandHost {
  /// Binds the Wayland globals and prepares an initially hidden menu.
  pub fn new(
    globals: GlobalList,
    qh: &QueueHandle<Self>,
    power_executor: LogindPowerExecutor,
    config: MenuConfig,
  ) -> Result<Self, io::Error> {
    let compositor_state = CompositorState::bind(&globals, qh)
      .expect("Wayland compositor global is unavailable");
    let layer_shell = LayerShell::bind(&globals, qh)
      .expect("the compositor does not support wlr-layer-shell");
    let shm_state = Shm::bind(&globals, qh).expect("wl_shm is unavailable");

    let last_size = config.frame_size();
    let (width, height) = last_size.dimensions();
    let pool = SlotPool::new(aligned_slot_bytes(last_size), &shm_state)
      .map_err(|err| match err {
        // The `Global` variant into an IO error with the
        // `Unsupported` kind. This is the most accurate kind to map to.
        CreatePoolError::Global(e) => {
          io::Error::new(io::ErrorKind::Unsupported, e)
        },
        // The `Create` variant already holds an IO error.
        CreatePoolError::Create(e) => e,
      })?;

    let view =
      MenuView::new(&config, width, height).map_err(io::Error::other)?;

    Ok(Self {
      registry_state: RegistryState::new(&globals),
      compositor_state,
      output_state: OutputState::new(&globals, qh),
      seat_state: SeatState::new(&globals, qh),
      shm_state,

      layer_shell,

      keyboard: None,
      pointer: None,
      keyboard_focus: KeyboardFocus::Unfocused,

      pool,
      view,

      daemon_state: DaemonState::Running,
      surface_state: MenuSurfaceState::Hidden { last_size },

      power_executor,
      controller: MenuController::new(config),
    })
  }

  /// Reports whether the daemon should continue after the current dispatch.
  pub(crate) fn is_running(&self) -> bool {
    self.daemon_state == DaemonState::Running
  }

  /// Requests that the daemon exit after the current dispatch.
  pub(crate) fn request_shutdown(&mut self) {
    self.daemon_state = DaemonState::Stopping;
  }

  /// Checks whether a layer exists, including one waiting for configure.
  fn is_menu_open(&self) -> bool {
    self.surface_state.layer().is_some()
  }

  /// Releases keyboard focus when its layer or keyboard is no longer active.
  fn clear_keyboard_focus(&mut self) {
    self.keyboard_focus = KeyboardFocus::Unfocused;
  }

  /// Handles socket commands and completed power requests on the main thread.
  pub fn handle_app_event(
    &mut self,
    event: AppEvent,
    qh: &QueueHandle<Self>,
  ) {
    let command = match event {
      AppEvent::DaemonCommand(command) => match command {
        DaemonCommand::Ping => return,
        DaemonCommand::Show => MenuCommand::Show,
        DaemonCommand::Hide => MenuCommand::Hide,
        DaemonCommand::Toggle => MenuCommand::Toggle,
        DaemonCommand::Quit => MenuCommand::Quit,
      },
      AppEvent::PowerActionFinished { action, result } => {
        MenuCommand::ActionFinished { action, result }
      },
    };
    self.handle_command(command, qh);
  }

  /// Runs the platform effect produced by a menu command.
  fn handle_command(
    &mut self,
    command: MenuCommand,
    qh: &QueueHandle<Self>,
  ) {
    match self.controller.update(command) {
      MenuEffect::None => {},
      MenuEffect::ShowMenu => self.show_menu(qh),
      MenuEffect::HideMenu => self.hide_menu(),
      MenuEffect::ToggleMenu => self.toggle_menu(qh),
      MenuEffect::RequestShutdown => self.request_shutdown(),
      MenuEffect::Redraw => self.draw(qh),
      MenuEffect::StartPowerAction(action) => {
        self.draw(qh);
        self.power_executor.execute(action);
      },
      MenuEffect::ReportFailureAndRedraw { action, error } => {
        eprintln!("failed to execute {action:?}: {error}");
        self.draw(qh);
      },
    }
  }

  /// Starts a layer surface when the menu is hidden.
  fn show_menu(&mut self, qh: &QueueHandle<Self>) {
    let MenuSurfaceState::Hidden { last_size } = self.surface_state else {
      return;
    };

    let layer = self.create_layer(qh);
    layer.commit();
    self.surface_state =
      MenuSurfaceState::WaitingForConfigure { layer, last_size };
  }

  /// Drops the current layer while preserving the selection and last size.
  fn hide_menu(&mut self) {
    if !self.is_menu_open() {
      return;
    }

    self.clear_keyboard_focus();
    self.surface_state = MenuSurfaceState::Hidden {
      last_size: self.surface_state.size(),
    };
  }

  /// Hides an open layer or starts a new one.
  fn toggle_menu(&mut self, qh: &QueueHandle<Self>) {
    if self.is_menu_open() {
      self.hide_menu();
    } else {
      self.show_menu(qh);
    }
  }

  /// Prepares view resources before committing the new settings.
  pub fn apply_config(
    &mut self,
    new_config: MenuConfig,
    qh: &QueueHandle<Self>,
  ) -> Result<(), AllocationError> {
    let size = self.surface_state.size();
    let size_changed = new_config.frame_size() != size;
    let (width, height) = size.dimensions();

    self.view.update_config(&new_config, width, height)?;
    self.controller.replace_config(new_config);

    match self.surface_state {
      MenuSurfaceState::Visible { .. }
      | MenuSurfaceState::WaitingForConfigure { .. }
        if size_changed =>
      {
        // Do a full close and open to reload UI, which
        // prevents lifecycle bugs.
        self.hide_menu();
        self.show_menu(qh);
      },
      // Visible but size did not change.
      MenuSurfaceState::Visible { .. } => self.draw(qh),
      // Otherwise, do nothing further.
      _ => {},
    }
    Ok(())
  }

  /// Renders and commits a frame for the configured layer.
  #[inline(never)]
  fn draw(&mut self, qh: &QueueHandle<Self>) {
    let MenuSurfaceState::Visible { layer, size } = &self.surface_state
    else {
      return;
    };
    let (width, height, stride) = size.buffer_shape();
    let required_len = size.byte_len();
    let slot_len = aligned_slot_bytes(*size);
    let pool_len = self.pool.len();
    if !pool_can_fit_slot(pool_len, slot_len) {
      return eprintln!(
        "shared-memory pool cannot safely grow for {width}x{height}"
      );
    }

    let frame = match self
      .view
      .render(self.controller.selected(), self.controller.config())
    {
      Ok(frame) => frame,
      Err(err) => {
        return eprintln!("failed to render {width}x{height}: {err}");
      },
    };

    let (buf, canvas) = match self.pool.create_buffer(
      width,
      height,
      stride,
      wl_shm::Format::Argb8888,
    ) {
      Ok(val) => val,
      Err(err) => {
        return eprintln!("failed to allocate Wayland buffer: {err}");
      },
    };

    rgba_to_argb8888(frame.as_rgba_bytes(), &mut canvas[..required_len]);

    let surface = layer.wl_surface();
    surface.damage_buffer(0, 0, width, height);

    // A frame callback is requested so compositor-driven redraws can be
    // handled through CompositorHandler::frame().
    //
    // `frame()` is currently empty, but this is kept in case animations
    // are added later on.
    surface.frame(qh, FrameCallbackData(surface.clone()));

    if let Err(err) = buf.attach_to(surface) {
      return eprintln!("failed to attach Wayland buffer: {err}");
    }

    surface.commit();
  }

  /// Creates the layer and sets its initial size and input behavior.
  fn create_layer(&self, qh: &QueueHandle<Self>) -> LayerSurface {
    let surface = self.compositor_state.create_surface(qh);
    let layer = self.layer_shell.create_layer_surface(
      qh,
      surface,
      Layer::Overlay,
      Some(crate::NAMESPACE),
      None,
    );
    layer.set_anchor(Anchor::empty());
    let (width, height) = self.controller.config().frame_size().dimensions();
    layer.set_size(width, height);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_exclusive_zone(-1);
    layer
  }

  /// Returns the current surface for filtering Wayland callbacks.
  fn wl_surface(&self) -> Option<&wl_surface::WlSurface> {
    self.surface_state.layer().map(|layer| layer.wl_surface())
  }
}

/// Packs premultiplied RGBA bytes into native-endian Wayland ARGB pixels.
fn rgba_to_argb8888(source: &[u8], destination: &mut [u8]) {
  debug_assert_eq!(source.len(), destination.len());
  debug_assert_eq!(source.len() % BYTES_PER_PIXEL, 0);

  for (rgba, argb) in source
    .chunks_exact(BYTES_PER_PIXEL)
    .zip(destination.chunks_exact_mut(BYTES_PER_PIXEL))
  {
    let packed = u32::from_be_bytes([rgba[3], rgba[0], rgba[1], rgba[2]]);
    argb.copy_from_slice(&packed.to_ne_bytes());
  }
}

/// Rounds a bounded frame allocation up to Smithay's slot alignment.
fn aligned_slot_bytes(size: FrameSize) -> usize {
  size.byte_len().next_multiple_of(64)
}

/// Checks Smithay's largest possible pool growth before it narrows to i32.
fn pool_can_fit_slot(pool_len: usize, slot_len: usize) -> bool {
  pool_len
    .checked_mul(2)
    .zip(pool_len.checked_add(slot_len))
    .is_some_and(|(doubled, extended)| {
      i32::try_from(doubled.max(extended)).is_ok()
    })
}

impl CompositorHandler for WaylandHost {
  fn scale_factor_changed(
    &mut self,
    _conn: &Connection,
    qh: &QueueHandle<Self>,
    surface: &wl_surface::WlSurface,
    _new_factor: i32,
  ) {
    if self.wl_surface() == Some(surface) {
      self.draw(qh);
    }
  }

  fn transform_changed(
    &mut self,
    _conn: &Connection,
    qh: &QueueHandle<Self>,
    surface: &wl_surface::WlSurface,
    _new_transform: wl_output::Transform,
  ) {
    if self.wl_surface() == Some(surface) {
      self.draw(qh);
    }
  }

  fn frame(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _surface: &wl_surface::WlSurface,
    _time: u32,
  ) {
    // The menu is static, so it does not need to redraw every frame.
  }

  fn surface_enter(
    &mut self,
    _connection: &Connection,
    _qh: &QueueHandle<Self>,
    _surface: &wl_surface::WlSurface,
    _output: &wl_output::WlOutput,
  ) {
  }

  fn surface_leave(
    &mut self,
    _connection: &Connection,
    _qh: &QueueHandle<Self>,
    _surface: &wl_surface::WlSurface,
    _output: &wl_output::WlOutput,
  ) {
  }
}

impl LayerShellHandler for WaylandHost {
  fn closed(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    layer: &LayerSurface,
  ) {
    if self.surface_state.layer() == Some(layer) {
      self.request_shutdown();
    }
  }

  fn configure(
    &mut self,
    _conn: &Connection,
    qh: &QueueHandle<Self>,
    layer: &LayerSurface,
    configure: LayerSurfaceConfigure,
    _serial: u32,
  ) {
    if self.surface_state.layer() != Some(layer) {
      return;
    }

    let config = self.controller.config();
    let (requested_width, requested_height) =
      config.frame_size().dimensions();
    let (width, height) = configure.new_size;
    let width = if width == 0 { requested_width } else { width };
    let height = if height == 0 {
      requested_height
    } else {
      height
    };
    let size = match FrameSize::new(width, height) {
      Ok(size) => size,
      Err(err) => {
        return eprintln!("invalid compositor size {width}x{height}: {err}");
      },
    };
    self.surface_state.configure(size);
    self.view.update_layout(config, width, height);
    self.draw(qh);
  }
}

impl OutputHandler for WaylandHost {
  fn output_state(&mut self) -> &mut OutputState {
    &mut self.output_state
  }

  fn new_output(
    &mut self,
    _connection: &Connection,
    _qh: &QueueHandle<Self>,
    _output: wl_output::WlOutput,
  ) {
  }

  fn update_output(
    &mut self,
    _connection: &Connection,
    _qh: &QueueHandle<Self>,
    _output: wl_output::WlOutput,
  ) {
  }

  fn output_destroyed(
    &mut self,
    _connection: &Connection,
    _qh: &QueueHandle<Self>,
    _output: wl_output::WlOutput,
  ) {
  }
}

impl SeatHandler for WaylandHost {
  fn seat_state(&mut self) -> &mut SeatState {
    &mut self.seat_state
  }

  fn new_seat(
    &mut self,
    _connection: &Connection,
    _qh: &QueueHandle<Self>,
    _seat: wl_seat::WlSeat,
  ) {
  }

  fn new_capability(
    &mut self,
    _conn: &Connection,
    qh: &QueueHandle<Self>,
    seat: wl_seat::WlSeat,
    capability: Capability,
  ) {
    match capability {
      Capability::Keyboard if self.keyboard.is_none() => {
        match self.seat_state.get_keyboard(qh, &seat, None) {
          Ok(kb) => self.keyboard = Some(kb),
          Err(e) => eprintln!("failed to create keyboard: {e}"),
        }
      },
      Capability::Pointer if self.pointer.is_none() => {
        match self.seat_state.get_pointer(qh, &seat) {
          Ok(ptr) => self.pointer = Some(ptr),
          Err(e) => eprintln!("failed to create pointer: {e}"),
        }
      },
      _ => {},
    };
  }

  fn remove_capability(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _seat: wl_seat::WlSeat,
    capability: Capability,
  ) {
    match capability {
      Capability::Keyboard if let Some(kb) = self.keyboard.take() => {
        kb.release();
        self.clear_keyboard_focus();
      },
      Capability::Pointer if let Some(ptr) = self.pointer.take() => {
        ptr.release();
      },
      _ => {},
    }
  }

  fn remove_seat(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _seat: wl_seat::WlSeat,
  ) {
  }
}

impl KeyboardHandler for WaylandHost {
  fn enter(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _keyboard: &wl_keyboard::WlKeyboard,
    surface: &wl_surface::WlSurface,
    _serial: u32,
    _raw: &[u32],
    _keysyms: &[Keysym],
  ) {
    if self.wl_surface() == Some(surface) {
      self.keyboard_focus = KeyboardFocus::Focused;
    }
  }

  fn leave(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _keyboard: &wl_keyboard::WlKeyboard,
    surface: &wl_surface::WlSurface,
    _serial: u32,
  ) {
    if self.wl_surface() == Some(surface) {
      self.clear_keyboard_focus();
    }
  }

  fn press_key(
    &mut self,
    _conn: &Connection,
    qh: &QueueHandle<Self>,
    _keyboard: &wl_keyboard::WlKeyboard,
    _serial: u32,
    event: KeyEvent,
  ) {
    if self.keyboard_focus != KeyboardFocus::Focused || !self.is_menu_open()
    {
      return;
    }

    let command = match event.keysym {
      Keysym::Escape => MenuCommand::Hide,
      Keysym::Up | Keysym::Left | Keysym::k | Keysym::h => {
        MenuCommand::Previous
      },
      Keysym::Down | Keysym::Right | Keysym::j | Keysym::l => {
        MenuCommand::Next
      },
      Keysym::Return | Keysym::KP_Enter | Keysym::space => {
        MenuCommand::ActivateSelected
      },
      Keysym::_1 => MenuCommand::Select(PowerAction::ALL[0]),
      Keysym::_2 => MenuCommand::Select(PowerAction::ALL[1]),
      Keysym::_3 => MenuCommand::Select(PowerAction::ALL[2]),
      Keysym::_4 => MenuCommand::Select(PowerAction::ALL[3]),
      Keysym::_5 => MenuCommand::Select(PowerAction::ALL[4]),
      Keysym::_6 => MenuCommand::Select(PowerAction::ALL[5]),
      _ => return,
    };
    self.handle_command(command, qh);
  }

  fn repeat_key(
    &mut self,
    _conn: &Connection,
    qh: &QueueHandle<Self>,
    _keyboard: &wl_keyboard::WlKeyboard,
    _serial: u32,
    event: KeyEvent,
  ) {
    if self.keyboard_focus != KeyboardFocus::Focused {
      return;
    }

    let command = match event.keysym {
      Keysym::Up | Keysym::Left | Keysym::k | Keysym::h => {
        MenuCommand::Previous
      },
      Keysym::Down | Keysym::Right | Keysym::j | Keysym::l => {
        MenuCommand::Next
      },
      _ => return,
    };
    self.handle_command(command, qh);
  }

  fn release_key(
    &mut self,
    _connection: &Connection,
    _qh: &QueueHandle<Self>,
    _keyboard: &wl_keyboard::WlKeyboard,
    _serial: u32,
    _event: KeyEvent,
  ) {
  }

  fn update_modifiers(
    &mut self,
    _connection: &Connection,
    _qh: &QueueHandle<Self>,
    _keyboard: &wl_keyboard::WlKeyboard,
    _serial: u32,
    _modifiers: Modifiers,
    _raw_modifiers: RawModifiers,
    _layout: u32,
  ) {
  }
}

impl PointerHandler for WaylandHost {
  fn pointer_frame(
    &mut self,
    _conn: &Connection,
    qh: &QueueHandle<Self>,
    _pointer: &wl_pointer::WlPointer,
    events: &[PointerEvent],
  ) {
    for event in events {
      if Some(&event.surface) != self.wl_surface() {
        continue;
      }

      let command = match event.kind {
        PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
          self.view.hit_test(event.position).map(MenuCommand::Select)
        },
        PointerEventKind::Press {
          button: BTN_LEFT, ..
        } => self
          .view
          .hit_test(event.position)
          .map(MenuCommand::Activate),
        _ => None,
      };
      if let Some(command) = command {
        self.handle_command(command, qh);
      }
    }
  }
}

impl ShmHandler for WaylandHost {
  fn shm_state(&mut self) -> &mut Shm {
    &mut self.shm_state
  }
}

delegate_registry!(WaylandHost);

impl ProvidesRegistryState for WaylandHost {
  fn registry(&mut self) -> &mut RegistryState {
    &mut self.registry_state
  }

  registry_handlers![OutputState, SeatState];
}

delegate_dispatch2!(WaylandHost);
