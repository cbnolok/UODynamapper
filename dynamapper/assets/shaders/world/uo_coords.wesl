// Rendering units only. Packed tile heights and gameplay coordinates stay in
// original signed UO Z units. Keep util_lib/uo_coords.rs in sync; GPU tests check
// the original reflection-height reconstruction against that CPU conversion.
const UO_Z_TO_WORLD_SCALE: f32 = 4.0 / 44.0;
