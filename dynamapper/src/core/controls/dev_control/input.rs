//! Ordered input for the app's primary window; no OS input/focus APIs are used.
use super::ControlCommand;
use crate::core::controls::InjectedPointer;
use bevy::{prelude::*, input::{ButtonState, keyboard::{Key, KeyCode, KeyboardInput, NativeKey, NativeKeyCode},
    mouse::{MouseButtonInput, MouseMotion, MouseScrollUnit, MouseWheel}}, window::{CursorMoved, WindowEvent}};
use bevy_egui::{EguiInput, PrimaryEguiContext};
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque};

const MAX_INPUT_FRAMES: usize = 256;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InputState { Press, Release, Tap }

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PointerButton { Left, Right, Middle, Back, Forward }

impl PointerButton {
    fn button(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left, Self::Right => MouseButton::Right,
            Self::Middle => MouseButton::Middle, Self::Back => MouseButton::Back,
            Self::Forward => MouseButton::Forward,
        }
    }
}

struct InputFrame { events: Vec<WindowEvent>, reset: bool }

#[derive(Resource, Default)]
pub(super) struct ControlInput {
    frames: VecDeque<InputFrame>,
    keys: HashMap<KeyCode, KeyboardInput>,
    buttons: HashSet<MouseButton>,
    pub focused: bool,
}

impl ControlInput {
    pub fn queued_frames(&self) -> usize { self.frames.len() }

    pub fn enqueue(&mut self, command: ControlCommand, window: Entity, bounds: Vec2) -> Result<(), String> {
        let pointer = |x: f32, y: f32| -> Result<WindowEvent, String> {
            let position = Vec2::new(x, y);
            if !position.is_finite() || x < 0.0 || y < 0.0 || x >= bounds.x || y >= bounds.y {
                return Err("pointer must be inside the primary window in logical pixels".into());
            }
            Ok(CursorMoved { window, position, delta: None }.into())
        };
        let keyboard = |key, text: Option<String>| -> Result<KeyboardInput, String> {
            if text.as_ref().is_some_and(|text| text.is_empty() || text.chars().any(char::is_control)) {
                return Err("text must be nonempty and contain no control characters; use named keys for Enter/Tab".into());
            }
            let logical_key = match text.as_ref() {
                Some(text) => Key::Character(text.as_str().into()),
                None if key == KeyCode::Space => Key::Space,
                None => Key::Unidentified(NativeKey::Unidentified),
            };
            Ok(KeyboardInput { key_code: key, logical_key, state: ButtonState::Pressed,
                text: text.map(Into::into), repeat: false, window })
        };
        let (events, state) = match command {
            ControlCommand::Key { key, state, text } => {
                if matches!(key, KeyCode::Unidentified(_)) { return Err("key requires a named Bevy KeyCode".into()); }
                (vec![keyboard(key, text)?.into()], state)
            }
            ControlCommand::Text { text } => (vec![keyboard(KeyCode::Unidentified(NativeKeyCode::Unidentified), Some(text))?.into()], InputState::Tap),
            ControlCommand::Pointer { x, y } => (vec![pointer(x, y)?], InputState::Press),
            ControlCommand::Click { x, y, button, state } => (vec![pointer(x, y)?, MouseButtonInput {
                button: button.button(), state: ButtonState::Pressed, window }.into()], state),
            ControlCommand::Scroll { x, y } => {
                if !x.is_finite() || !y.is_finite() { return Err("scroll deltas must be finite".into()); }
                (vec![MouseWheel { unit: MouseScrollUnit::Line, x, y, window,
                    phase: bevy::input::touch::TouchPhase::Moved }.into()], InputState::Press)
            }
            ControlCommand::ResetInput {} => {
                let mut events = Vec::new();
                for key in self.keys.values() { let mut key = key.clone(); key.state = ButtonState::Released; key.text = None; events.push(key.into()); }
                for button in &self.buttons { events.push(MouseButtonInput { button: *button, state: ButtonState::Released, window }.into()); }
                self.frames.clear();
                self.frames.push_back(InputFrame { events, reset: true });
                return Ok(());
            }
            _ => return Err("not an input command".into()),
        };
        let count = if matches!(state, InputState::Tap) { 2 } else { 1 };
        if self.frames.len() + count > MAX_INPUT_FRAMES { return Err("input queue is full; wait or reset_input".into()); }
        let released = |event: &WindowEvent| match event {
            WindowEvent::KeyboardInput(key) => { let mut key = key.clone(); key.state = ButtonState::Released; key.text = None; key.into() }
            WindowEvent::MouseButtonInput(button) => { let mut button = *button; button.state = ButtonState::Released; button.into() }
            event => event.clone(),
        };
        match state {
            InputState::Press => self.frames.push_back(InputFrame { events, reset: false }),
            InputState::Release => self.frames.push_back(InputFrame { events: events.iter().map(released).collect(), reset: false }),
            InputState::Tap => {
                let release = events.iter().filter(|event| matches!(event, WindowEvent::KeyboardInput(_) | WindowEvent::MouseButtonInput(_))).map(released).collect();
                self.frames.push_back(InputFrame { events, reset: false });
                self.frames.push_back(InputFrame { events: release, reset: false });
            }
        }
        Ok(())
    }
}

pub(super) fn inject_control_input(
    mut input: ResMut<ControlInput>,
    mut pointer: ResMut<InjectedPointer>,
    mut keys: MessageWriter<KeyboardInput>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut moved: MessageWriter<CursorMoved>,
    mut motion: MessageWriter<MouseMotion>,
    mut scroll: MessageWriter<MouseWheel>,
    mut ordered: MessageWriter<WindowEvent>,
) {
    let Some(frame) = input.frames.pop_front() else { return; };
    input.focused = !frame.reset;
    for mut event in frame.events {
        match &mut event {
            WindowEvent::KeyboardInput(key) => {
                if key.state.is_pressed() { input.keys.insert(key.key_code, key.clone()); }
                else { input.keys.remove(&key.key_code); }
                keys.write(key.clone());
            }
            WindowEvent::MouseButtonInput(button) => {
                if button.state.is_pressed() { input.buttons.insert(button.button); }
                else { input.buttons.remove(&button.button); }
                buttons.write(*button);
            }
            WindowEvent::CursorMoved(event) => {
                event.delta = pointer.0.map(|previous| event.position - previous);
                if let Some(delta) = event.delta { motion.write(MouseMotion { delta }); }
                pointer.0 = Some(event.position);
                moved.write(event.clone());
            }
            WindowEvent::MouseWheel(event) => { scroll.write(*event); }
            _ => unreachable!("control input contains only supported input events"),
        }
        // egui 0.43 reads pointer movement/buttons from this ordered stream.
        ordered.write(event);
    }
    if frame.reset { pointer.0 = None; }
}

pub(super) fn focus_control_context(input: Res<ControlInput>, mut contexts: Query<&mut EguiInput, With<PrimaryEguiContext>>) {
    if input.focused {
        for mut context in &mut contexts { context.focused = true; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::InputPlugin;
    use bevy::window::PrimaryWindow;
    use bevy_egui::{EguiGlobalSettings, input::{EguiInputEvent, EguiFileDragAndDropMessage,
        ModifierKeysState, WindowToEguiContextMap, write_pointer_moved_and_button_messages_system,
        write_egui_input_system}};

    fn input_app() -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins((InputPlugin, bevy::time::TimePlugin))
            .add_message::<CursorMoved>().add_message::<WindowEvent>()
            .add_message::<EguiInputEvent>().add_message::<EguiFileDragAndDropMessage>()
            .init_resource::<ControlInput>().init_resource::<InjectedPointer>()
            .init_resource::<EguiGlobalSettings>().init_resource::<ModifierKeysState>()
            .init_resource::<WindowToEguiContextMap>()
            .add_systems(PreUpdate, inject_control_input.before(bevy::input::InputSystems))
            .add_systems(PreUpdate, (write_pointer_moved_and_button_messages_system,
                write_egui_input_system, focus_control_context).chain().after(bevy::input::InputSystems));
        let window = app.world_mut().spawn((Window { focused: false, resolution: (640, 480).into(), ..default() }, PrimaryWindow)).id();
        let context = app.world_mut().spawn((PrimaryEguiContext, Camera::default())).id();
        let mut map = app.world_mut().resource_mut::<WindowToEguiContextMap>();
        map.window_to_contexts.entry(window).or_default().insert(context);
        map.context_to_window.insert(context, window);
        (app, window, context)
    }

    fn enqueue(app: &mut App, window: Entity, json: &str) {
        let command = serde_json::from_str(json).unwrap();
        app.world_mut().resource_mut::<ControlInput>().enqueue(command, window, Vec2::new(640.0, 480.0)).unwrap();
    }

    #[test]
    fn key_taps_reach_input_state_on_separate_frames_and_reset_releases_held_input() {
        let (mut app, window, _) = input_app();
        enqueue(&mut app, window, r#"{"command":"key","key":"ControlLeft","state":"press"}"#);
        enqueue(&mut app, window, r#"{"command":"key","key":"KeyG","state":"tap"}"#);
        app.update();
        assert!(app.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::ControlLeft));
        app.update();
        assert!(app.world().resource::<ButtonInput<KeyCode>>().just_pressed(KeyCode::KeyG));
        app.update();
        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(keys.pressed(KeyCode::ControlLeft));
        assert!(keys.just_released(KeyCode::KeyG));
        enqueue(&mut app, window, r#"{"command":"click","x":20,"y":30,"button":"right","state":"press"}"#);
        app.update();
        assert!(app.world().resource::<ButtonInput<MouseButton>>().pressed(MouseButton::Right));
        enqueue(&mut app, window, r#"{"command":"key","key":"KeyW","state":"tap"}"#);
        enqueue(&mut app, window, r#"{"command":"reset_input"}"#);
        app.update();
        assert!(!app.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::ControlLeft));
        assert!(!app.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::KeyW));
        assert!(!app.world().resource::<ButtonInput<MouseButton>>().pressed(MouseButton::Right));
        assert_eq!(app.world().resource::<InjectedPointer>().0, None);
        assert!(!app.world().resource::<ControlInput>().focused);
    }

    #[test]
    fn clicks_preserve_egui_position_order_and_leave_window_focus_and_cursor_untouched() {
        let (mut app, window, context) = input_app();
        enqueue(&mut app, window, r#"{"command":"click","x":27,"y":31,"button":"left","state":"tap"}"#);
        app.update();
        app.update();
        let input = app.world().get::<EguiInput>(context).unwrap();
        assert!(matches!(input.events.as_slice(), [
            bevy_egui::egui::Event::PointerMoved(position),
            bevy_egui::egui::Event::PointerButton { pos, pressed: true, .. },
            bevy_egui::egui::Event::PointerButton { pos: release, pressed: false, .. },
        ] if *position == bevy_egui::egui::pos2(27.0, 31.0) && position == pos && pos == release));
        assert!(input.focused);
        assert!(!app.world().get::<Window>(window).unwrap().focused);
        assert_eq!(app.world().get::<Window>(window).unwrap().cursor_position(), None);
        assert_eq!(app.world().resource::<InjectedPointer>().0, Some(Vec2::new(27.0, 31.0)));
        enqueue(&mut app, window, r#"{"command":"reset_input"}"#);
        app.update();
        assert!(!app.world().get::<EguiInput>(context).unwrap().focused);
    }

    #[test]
    fn invalid_pointer_text_and_full_queue_do_not_partially_apply_input() {
        let (mut app, window, _) = input_app();
        for json in [r#"{"command":"pointer","x":640,"y":0}"#,
            r#"{"command":"click","x":-1,"y":30,"button":"left","state":"tap"}"#,
            r#"{"command":"text","text":"\n"}"#] {
            let command = serde_json::from_str(json).unwrap();
            assert!(app.world_mut().resource_mut::<ControlInput>().enqueue(command, window, Vec2::new(640.0, 480.0)).is_err());
        }
        assert_eq!(app.world().resource::<ControlInput>().queued_frames(), 0);
        for _ in 0..MAX_INPUT_FRAMES { enqueue(&mut app, window, r#"{"command":"pointer","x":0,"y":0}"#); }
        let command = serde_json::from_str(r#"{"command":"key","key":"F2","state":"tap"}"#).unwrap();
        assert!(app.world_mut().resource_mut::<ControlInput>().enqueue(command, window, Vec2::new(640.0, 480.0)).is_err());
        assert_eq!(app.world().resource::<ControlInput>().queued_frames(), MAX_INPUT_FRAMES);
        enqueue(&mut app, window, r#"{"command":"reset_input"}"#);
        app.update();
        assert_eq!(app.world().resource::<ControlInput>().queued_frames(), 0);
    }
}
