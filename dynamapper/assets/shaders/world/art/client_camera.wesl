// ============================================================================
// art::client_camera — the Kingdom Reborn client camera's constants that its shadows (client_shadow.wesl) and
// grunge (client_grunge.wesl) share. No bindings: the sprite and the ground art shaders both import it.
// ============================================================================

// World units per UO z unit along the camera's up axis, 0x00b86f38 (= 6 K, K = 0.021427422 per art pixel).
const CLIENT_Z_UNIT: f32 = 0.12856454;
