use bevy::prelude::*;

/// Action to toggle the keybindings help dialog (F1)
#[derive(Event)]
pub struct ActionToggleKeybindingsHelp;

/// Action to toggle the preferences/options dialog (F2)
#[derive(Event)]
pub struct ActionTogglePreferences;

/// Action to toggle the terrain shader settings dialog (F3)
#[derive(Event)]
pub struct ActionToggleShaderSettings;

/// Action to toggle fullscreen (F11 / Alt+Enter)
#[derive(Event)]
pub struct ActionToggleFullscreen;

/// Action to toggle the teleport coordinate dialog (Ctrl + G)
#[derive(Event)]
pub struct ActionToggleTeleportDialog;

/// Action to toggle the cursor teleport mode (Ctrl + T)
#[derive(Event)]
pub struct ActionToggleCursorTeleportMode;

/// Action to toggle the cursor tile inspection panel (I)
#[derive(Event)]
pub struct ActionToggleCursorInspectPanel;

/// Action to close the active dialog (Escape)
#[derive(Event)]
pub struct ActionCloseActiveDialog;
