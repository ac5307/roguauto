// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

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

use std::{num::NonZeroU32 as NZU32};

use crate::{
  async_event::AppEvent,
  config::deserialize::Configuration,
  daemon::command::DaemonCommand,
  io,
  power::{PowerAction, PowerHandler},
  render::{rgba_to_argb8888, item_at, Renderer},
};

const BYTES_PER_PIXEL: usize = tiny_skia::BYTES_PER_PIXEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuState {
  Hidden,
  WaitingForConfigure,
  Visible,
}

pub struct WaylandApp {
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
  layer: Option<LayerSurface>,

  /*
   * Input objects.
   */
  keyboard: Option<wl_keyboard::WlKeyboard>,
  pointer: Option<wl_pointer::WlPointer>,
  keyboard_focus: bool,
  pointer_position: Option<(f64, f64)>,

  /*
   * Shared-memory rendering.
   */
  pool: SlotPool,
  renderer: Renderer,

  /*
   * Current configured surface size.
   */
  width: u32,
  height: u32,

  /*
   * Application services.
   */
  power_handler: PowerHandler,
  config: Configuration,

  /*
   * Daemon/UI state.
   */
  pub running: bool,
  menu_state: MenuState,
  selected: usize,
  action_in_flight: bool,
}

impl WaylandApp {
  pub fn new(
    globals: GlobalList,
    qh: &QueueHandle<Self>,
    power_handler: PowerHandler,
    config: Configuration,
  ) -> Result<Self, io::Error> {
    let compositor_state = CompositorState::bind(&globals, qh)
      .expect("Wayland compositor global is unavailable");
    let layer_shell = LayerShell::bind(&globals, qh)
      .expect("the compositor does not support wlr-layer-shell");
    let shm_state = Shm::bind(&globals, qh).expect("wl_shm is unavailable");

    let width = config.window_width().max(1);
    let height = config.window.height.max(1);

    let initial_pool_size = required_buffer_bytes(width, height)?;
    let pool = SlotPool::new(initial_pool_size, &shm_state).map_err(
      |err| match err {
        // The `Global` variant into an IO error with the
        // `Unsupported` kind. This is the most accurate kind to map to.
        CreatePoolError::Global(e) => {
          io::Error::new(io::ErrorKind::Unsupported, e)
        },
        // The `Create` variant already holds an IO error.
        CreatePoolError::Create(e) => e,
      },
    )?;

    let renderer = Renderer::new(&config);

    Ok(Self {
      registry_state: RegistryState::new(&globals),
      compositor_state,
      output_state: OutputState::new(&globals, qh),
      seat_state: SeatState::new(&globals, qh),
      shm_state,

      layer_shell,
      layer: None,

      keyboard: None,
      pointer: None,
      keyboard_focus: false,
      pointer_position: None,

      pool,
      renderer,

      width,
      height,

      running: true,

      menu_state: MenuState::Hidden,

      selected: 0,
      action_in_flight: false,

      power_handler,
      config,
    })
  }

  #[inline]
  pub fn config(&self) -> &Configuration {
    &self.config
  }

  #[inline]
  pub fn is_visible(&self) -> bool {
    self.menu_state != MenuState::Hidden
  }

  #[inline]
  pub fn action_in_flight(&self) -> bool {
    self.action_in_flight
  }

  #[inline]
  pub fn selected(&self) -> usize {
    self.selected
  }

  #[inline]
  pub fn selected_action(&self) -> PowerAction {
    PowerAction::ALL[self.selected]
  }

  pub fn handle_app_event(
    &mut self,
    event: AppEvent,
    qh: &QueueHandle<Self>,
  ) {
    match event {
      AppEvent::DaemonCommand(cmd) => match cmd {
        DaemonCommand::Ping => {},
        DaemonCommand::Show => {
          self.show_menu(qh);
        },
        DaemonCommand::Hide => {
          self.hide_menu();
        },
        DaemonCommand::Toggle => {
          self.toggle_menu(qh);
        },
        DaemonCommand::Quit => {
          self.running = false;
        },
      },
      AppEvent::PowerActionFinished { action, result } => {
        self.action_in_flight = false;

        match result {
          Ok(_) => {
            if self.config.behavior.close_after_action {
              self.hide_menu();
            } else {
              self.draw(qh);
            }
          },
          Err(err) => {
            eprintln!("failed to execute {action:?}: {err}");
            self.draw(qh);
          },
        }
      },
    };
  }

  pub fn show_menu(&mut self, qh: &QueueHandle<Self>) {
    match self.menu_state {
      MenuState::Visible | MenuState::WaitingForConfigure => return,
      MenuState::Hidden => {},
    };

    self.selected = self.selected.min(PowerAction::ALL.len() - 1);

    self.layer = Some(self.create_layer(qh));

    self.menu_state = MenuState::WaitingForConfigure;

    self.layer().commit();
  }

  pub fn hide_menu(&mut self) {
    if self.menu_state == MenuState::Hidden {
      return;
    }

    self.menu_state = MenuState::Hidden;
    self.keyboard_focus = false;
    self.pointer_position = None;

    self.layer = None;
  }

  pub fn toggle_menu(&mut self, qh: &QueueHandle<Self>) {
    if self.is_visible() {
      self.hide_menu();
    } else {
      self.show_menu(qh);
    }
  }

  pub fn apply_config(
    &mut self,
    new_config: Configuration,
    qh: &QueueHandle<Self>,
  ) {
    let new_width = new_config.window_width();
    let new_height = new_config.window.height;

    let size_changed = new_width != self.width || new_height != self.height;

    self.config = new_config;
    self.renderer.update_config(&self.config);

    match self.menu_state {
      MenuState::Visible | MenuState::WaitingForConfigure
        if size_changed =>
      {
        // Do a full close and open to reload UI, which
        // prevents lifecycle bugs.
        self.hide_menu();
        self.show_menu(qh);
      },
      // Visible but size did not change.
      MenuState::Visible => self.draw(qh),
      // Otherwise, do nothing further.
      _ => {},
    }
  }

  fn select_previous(&mut self, qh: &QueueHandle<Self>) {
    if self.action_in_flight {
      return;
    }

    self.selected = match self.selected.checked_sub(1) {
      Some(prev) => prev,
      _ => PowerAction::ALL.len() - 1,
    };

    self.draw(qh);
  }

  fn select_next(&mut self, qh: &QueueHandle<Self>) {
    if self.action_in_flight {
      return;
    }

    self.selected = (self.selected + 1) % PowerAction::ALL.len();

    self.draw(qh);
  }

  fn set_selected(&mut self, selected: usize, qh: &QueueHandle<Self>) {
    if self.action_in_flight
      || selected >= PowerAction::ALL.len()
      || self.selected == selected
    {
      return;
    }

    self.selected = selected;
    self.draw(qh);
  }

  fn activate_selected(&mut self, qh: &QueueHandle<Self>) {
    if self.action_in_flight {
      return;
    }

    let action = self.selected_action();

    // Redraws
    self.action_in_flight = true;
    self.draw(qh);

    // Execute the action.
    self.power_handler.execute(action);
  }

  fn hit_test(&self, pos: (f64, f64)) -> Option<usize> {
    let (x, y) = pos;

    if !x.is_finite()
      || !y.is_finite()
      || x < 0.0
      || y < 0.0
      || x >= self.width as f64
      || y >= self.height as f64
    {
      return None;
    }

    item_at(x, y, self.height, &self.config)
  }

  #[inline(never)]
  fn draw(&mut self, qh: &QueueHandle<Self>) {
    // VERY IMPORTANT condition to verify before continuing.
    if self.menu_state != MenuState::Visible {
      return;
    }

    let width = self.width;
    let height = self.height;

    let Ok(required_len) = required_buffer_bytes(width, height) else {
      return eprintln!(
        "surface dimensions are too large: {width}x{height}"
      );
    };

    let Ok(stride) = buffer_stride(width) else {
      return eprintln!("surface is too large: {width}");
    };

    if self.pool.len() < required_len
      && let Err(err) = self.pool.resize(required_len)
    {
      return eprintln!("failed to resize shared-memory pool: {err}");
    }

    let (buf, canvas) = match self.pool.create_buffer(
      width as i32,
      height as i32,
      stride,
      wl_shm::Format::Argb8888,
    ) {
      Ok(val) => val,
      Err(err) => {
        return eprintln!("failed to allocate Wayland buffer: {err}");
      },
    };

    let Some(pixmap) = self.renderer.render(
      width,
      height,
      self.selected,
      self.action_in_flight,
      &self.config,
    ) else {
      return eprintln!(
        "failed to create renderer pixmap for {width}x{height}"
      );
    };

    // Copy
    rgba_to_argb8888(pixmap.data(), &mut canvas[..required_len]);

    let surface = self.layer().wl_surface();
    surface.damage_buffer(0, 0, width as i32, height as i32);

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
    layer.set_size(
      self.config.window_width().max(1),
      self.config.window.height.max(1),
    );
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_exclusive_zone(-1);
    layer
  }

  #[inline]
  fn wl_surface(&self) -> Option<&wl_surface::WlSurface> {
    self.layer.as_ref().map(|layer| layer.wl_surface())
  }

  #[inline]
  fn layer(&self) -> &LayerSurface {
    self
      .layer
      .as_ref()
      .expect("layer should exist while visible")
  }
}

fn buffer_stride(width: u32) -> Result<i32, io::Error> {
  use io::{Error, ErrorKind};
  let stride = usize::try_from(width)
    .ok()
    .and_then(|width| width.checked_mul(BYTES_PER_PIXEL))
    .ok_or_else(|| {
      Error::new(ErrorKind::InvalidInput, "buffer stride overflow")
    })?;

  i32::try_from(stride).map_err(|_| {
    Error::new(ErrorKind::InvalidInput, "buffer stride exceeds i32")
  })
}

fn required_buffer_bytes(
  width: u32,
  height: u32,
) -> Result<usize, io::Error> {
  use io::{Error, ErrorKind};
  let width = usize::try_from(width).map_err(|_| {
    Error::new(ErrorKind::InvalidInput, "buffer width exceeds usize")
  })?;

  let height = usize::try_from(height).map_err(|_| {
    Error::new(ErrorKind::InvalidInput, "buffer height exceeds usize")
  })?;

  width
    .checked_mul(height)
    .and_then(|pixels| pixels.checked_mul(BYTES_PER_PIXEL))
    .ok_or_else(|| {
      Error::new(ErrorKind::InvalidInput, "buffer size overflow")
    })
}

impl CompositorHandler for WaylandApp {
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

impl LayerShellHandler for WaylandApp {
  fn closed(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    layer: &LayerSurface,
  ) {
    if self.layer.as_ref() == Some(layer) {
      self.running = false;
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
    if self.layer.as_ref() != Some(layer) {
      return;
    }

    let cfg_width = self.config.window_width().max(1);
    let cfg_height = self.config.window.height.max(1);

    self.width =
      NZU32::new(configure.new_size.0).map_or(cfg_width, NZU32::get);
    self.height =
      NZU32::new(configure.new_size.1).map_or(cfg_height, NZU32::get);

    match self.menu_state {
      MenuState::Hidden => return,
      MenuState::WaitingForConfigure => self.menu_state = MenuState::Visible,
      MenuState::Visible => {},
    };
    self.draw(qh);
  }
}

impl OutputHandler for WaylandApp {
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

impl SeatHandler for WaylandApp {
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
        self.keyboard_focus = false
      },
      Capability::Pointer if let Some(ptr) = self.pointer.take() => {
        ptr.release();
        self.pointer_position = None;
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

impl KeyboardHandler for WaylandApp {
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
      self.keyboard_focus = true
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
      self.keyboard_focus = false;
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
    if !self.keyboard_focus || !self.is_visible() {
      return;
    }

    match event.keysym {
      Keysym::Escape => self.hide_menu(),
      Keysym::Up | Keysym::Left | Keysym::k | Keysym::h => {
        self.select_previous(qh);
      },
      Keysym::Down | Keysym::Right | Keysym::j | Keysym::l => {
        self.select_next(qh);
      },
      Keysym::Return | Keysym::KP_Enter | Keysym::space => {
        self.activate_selected(qh);
      },
      Keysym::_1 => {
        self.set_selected(0, qh);
      },
      Keysym::_2 => {
        self.set_selected(1, qh);
      },
      Keysym::_3 => {
        self.set_selected(2, qh);
      },
      Keysym::_4 => {
        self.set_selected(3, qh);
      },
      Keysym::_5 => {
        self.set_selected(4, qh);
      },
      Keysym::_6 => {
        self.set_selected(5, qh);
      },
      _ => {},
    }
  }

  fn repeat_key(
    &mut self,
    _conn: &Connection,
    qh: &QueueHandle<Self>,
    _keyboard: &wl_keyboard::WlKeyboard,
    _serial: u32,
    event: KeyEvent,
  ) {
    if !self.keyboard_focus {
      return;
    }

    // Only navigation may repeat.
    match event.keysym {
      Keysym::Up | Keysym::Left | Keysym::k | Keysym::h => {
        self.select_previous(qh);
      },
      Keysym::Down | Keysym::Right | Keysym::j | Keysym::l => {
        self.select_next(qh);
      },
      _ => {},
    }
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

impl PointerHandler for WaylandApp {
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

      match event.kind {
        PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
          if !self.is_visible() || self.action_in_flight {
            continue;
          }

          self.pointer_position = Some(event.position);

          let Some(idx) = self.hit_test(event.position) else {
            continue;
          };
          self.set_selected(idx, qh);
        },
        PointerEventKind::Leave { .. } => {
          self.pointer_position = None;
        },
        PointerEventKind::Press { button, .. } => {
          if button != BTN_LEFT
            || !self.is_visible()
            || self.action_in_flight
          {
            continue;
          }

          let Some(idx) = self.hit_test(event.position) else {
            continue;
          };

          self.selected = idx;
          self.activate_selected(qh);
        },
        _ => {},
      }
    }
  }
}

impl ShmHandler for WaylandApp {
  fn shm_state(&mut self) -> &mut Shm {
    &mut self.shm_state
  }
}

delegate_registry!(WaylandApp);

impl ProvidesRegistryState for WaylandApp {
  fn registry(&mut self) -> &mut RegistryState {
    &mut self.registry_state
  }

  registry_handlers![OutputState, SeatState];
}

delegate_dispatch2!(WaylandApp);
