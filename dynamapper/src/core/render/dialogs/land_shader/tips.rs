//! Hover texts for the Land Shader Controls window and the Advanced Raw Shader Knobs window.
//!
//! One text per setting: where a setting has a control in both windows, both use the same text. Each says what the
//! control does, which presets and times of day show it, what it needs, and whether it acts with only one of the
//! two texture sets (Classic `.mul` or Enhanced `.uop` textures). The facts come from the measurements in
//! `docs/agent_notes/shader_settings_audit.md`.

// --- Preset pickers and window buttons ---

pub(super) const TIME_OF_DAY: &str =
    "Loads this render style's preset for the time of day. Your edits to the current preset are kept until you save or undo.";
pub(super) const SHOW_ADVANCED: &str =
    "Opens every raw shader value in a second window. Its controls edit the same values as the ones here.";
pub(super) const RESET_PRESET: &str =
    "Puts the current render style and time of day back to the factory values, and in KR-like and Enhanced the style's Local Lights, shared by every time of day.";
pub(super) const UNDO_TO_SAVED: &str =
    "Restores the values saved when this window was last closed.";

// --- Whole scene ---

pub(super) const TONEMAP: &str =
    "Compresses bright light so sunlit areas keep their detail instead of washing out. On in every preset; turning it off brightens the frame, most in Classic and Enhanced.";
pub(super) const TONE_CURVE: &str =
    "Which tone curve Tonemap uses. Needs Tonemap on.";
pub(super) const COLOR_GRADING: &str =
    "Turns on the colour grading tuned under Advanced > Color Grading, in every render style; on in UOD-Painterly, and in KR-like for the Chroma Knee alone. If colours look oversaturated, lower Saturation, Vibrance or the Chroma Knee there.";
pub(super) const TEXTURE_SMOOTHNESS: &str =
    "Softens the seams between texture pixels under the Pixel art filter; liquids and item surfaces follow it under either filter. On in KR-like and UOD-Painterly.";
pub(super) const TEXTURE_FILTER: &str =
    "How land and item textures are sampled: crisp pixel art, or the KR client's plain bilinear (KR-like).";
pub(super) const FOG: &str =
    "Adds haze toward the screen edges and the bottom, with slowly drifting cloud patches. On in UOD-Painterly; shape it under Advanced > Fog Params.";
pub(super) const BRIGHTNESS: &str =
    "Overall light level of the scene in Classic and UOD-Painterly, whose presets lower it at night and in caves; KR-like and Enhanced dim through Light Level instead. The darker the scene, the more light sources glow.";
pub(super) const LIGHT_STATIC_INTERACTION: &str =
    "How brightly lamps, torches and other light sources light the ground around them, at any time of day. Visible in Classic and UOD-Painterly, off in Enhanced and KR-like.";
pub(super) const GRUNGE: &str =
    "Lays a dirt and wear texture over land and items. Strong in UOD-Painterly at every time; the Enhanced amount is too light to see.";
pub(super) const KR_COLOR_WASH: &str =
    "Adds large, soft patches of colour variation over land and items, like a painted wash. Needs the KR visual profile (KR-like, UOD-Painterly); at the UOD-Painterly amount it is very faint, from about 1.5 it shows as broad blotches.";
pub(super) const KR_COLOR_WASH_SCALE: &str =
    "Size of the colour wash patches; larger values make bigger patches. Needs KR Color Wash on and matters only at a strong wash.";

// --- Client night lighting and shadows (KR-like, Enhanced) ---

pub(super) const CLIENT_LIGHT_LEVEL: &str =
    "The server's light level: 0 is full day (no change), about 10 a night, 26 a dungeon. KR-like and Enhanced, in place of Brightness.";
pub(super) const NIGHT_OVERLAY_STRENGTH: &str =
    "The night map's tint. 1 is the client; 0 darkens as much without the blue; above 1 tints more. Needs a Light Level above 0.";
pub(super) const NIGHT_OVERLAY_COLOR: &str =
    "The night map's mean colour. The client's keeps its blue; another colour keeps its cloud pattern.";
pub(super) const NIGHT_OVERLAY_CLIENT: &str =
    "Back to the client's colour, the night map's exact mean.";
pub(super) const LIGHT_COVER: &str =
    "Which rule hides the lights a roof or an upper floor covers, in this style at every time of day. Both look at the cell in front of a light, toward the camera: a room's lights go under its roof, a light on an outer wall facing you stays. The light's glow and the local lights taken from it go together.";
pub(super) const LIGHT_COVER_CLASSICUO: &str =
    "ClassicUO's rule: an opaque static at least 5 z above the light, in the cell in front of it, hides it. Dynamapper keeps the lights hung under an eave lit: when no wall stands at the light's height in front of it and the cover ends within two more cells toward the camera, the light is outside the building.";
pub(super) const LIGHT_COVER_CLIENTS: &str =
    "The Enhanced and KR clients' rule: anything in the cell in front of the light that sits in a higher 20-z band above the land hides it, with no exception for eaves or see-through items. A roof 20 z above always hides; one 5 to 19 z above hides when it crosses a band. KR-like follows the KR client, counting each band from the land under the light or the cover, and a floor or roof 5 z or more above the light hides it too; the other styles follow the Enhanced client, counting both from the land under the light.";
pub(super) const LOCAL_LIGHT_SOURCE: &str =
    "Which lights light the ground through Light/Static Interaction in KR-like and Enhanced, and the statics too when world_rendering.enable_static_art_local_lights is on. Light Sprites: the client light sprites, each lighting the ground under its glow, as large as the sprite and in its texture's colour. light.mul: the Classic lights, as in Classic and UOD-Painterly: every static with the light source flag, with its light.mul light's size and its hue or light colour. Either way the night still glows with the client light sprites. Needs Light/Static Interaction above 0 (0 in these presets). One choice for the style at every time of day, and each source keeps its own intensity, the slider below.";
pub(super) const LOCAL_LIGHTS_LIGHT_SPRITES: &str =
    "The client light sprites light the ground under their glow, as large as the sprite and in its texture's colour.";
pub(super) const LOCAL_LIGHTS_LIGHT_MUL: &str =
    "The Classic lights from light.mul, as in Classic: every static with the light source flag, with its light's size and its hue or light colour.";
pub(super) const LIGHT_SPRITES_INTENSITY: &str =
    "How strongly the light sprites light the ground and the statics, on top of Light/Static Interaction: 1 as they come, 0 off, up to 4. One value for the style at every time of day; light.mul keeps its own. The light sprites' glow in the night stays as it is.";
pub(super) const LIGHT_MUL_INTENSITY: &str =
    "How strongly the light.mul lights light the ground and the statics, on top of Light/Static Interaction: 1 as they come, 0 off, up to 4. light.mul lights fewer statics and smaller pools than the light sprites, so at 1 it lights less overall; raise this to even them out where both have a light. One value for the style at every time of day; the light sprites keep their own. The light sprites' glow in the night stays as it is.";
pub(super) const CENTRE_LIGHTS_ON_ITEMS: &str =
    "Moves the light sprites the client places far from their item onto the item's tile, keeping their height on screen, and the ground light taken from them with them: lava, fires, hearths and some sconces and windows, whose glow the client throws about five tiles aside. The sprites the client places near their item (lanterns, wall torches, window beams) stay. Off is the client's placement. light.mul lights sit on their item's tile either way. One choice for the style at every time of day.";
pub(super) const CLIENT_SHADOWS: &str =
    "The KR client's shadows of statics and items, leaning from their base (its Show Shadows option). KR-like only, where they replace Shadow Shape, Contact Shadows and Projected Shadows.";
pub(super) const CLIENT_GRUNGE: &str =
    "The KR client's grunge: darkens statics and items it marks with soft blotches of its grunge texture, repeating every 16 tiles, never on land or light glows. The same blotches fall on Classic Client and Enhanced Client art sources. Strength 1 is the client; on in KR-like, optional in every style. Needs tex_land_kr.uddp for KR's texture, else uses the Enhanced Client's, a different image.";
pub(super) const CLIENT_GRUNGE_STRENGTH: &str =
    "How much Client Grunge darkens: 1 is the KR client's full multiply, 0 leaves the colours as they are.";

// --- Terrain and water ---

pub(super) const WATER_ANIMATION: &str =
    "Moves the water surfaces of land and statics. Water only; it keeps its value when you switch presets and stops when zoomed far in.";
pub(super) const LAND_SHADING: &str =
    "How Enhanced and KR styles light land: smoothly per pixel (UOD-Painterly), or the clients' per-corner grey (KR-like, Enhanced). Classic always uses its own per-corner light.";
pub(super) const SUN_STRENGTH: &str =
    "How strongly the sun lights land that faces it: the main light and shade on terrain in Classic and UOD-Painterly. With Client per-vertex land shading (KR-like, Enhanced) it lights only water: land water, flat-water statics and water drawn from Enhanced textures.";
pub(super) const SUN_AZIMUTH: &str =
    "Compass bearing of the sun in degrees, clockwise from north (180 = south). Lights land and art and turns the sprite shadows.";
pub(super) const SUN_ELEVATION: &str =
    "Height of the sun above the horizon in degrees (90 = overhead).";
pub(super) const TERRAIN_AMBIENT: &str =
    "Light that reaches every land tile, even in shade, in Classic and with per-pixel land shading; raise it to open up dark slopes, nights and caves. With Client per-vertex land shading it lights only water: land water, flat-water statics and water drawn from Enhanced textures; KR-like uses it to draw water unlit, as the client.";
pub(super) const LAND_RELIEF: &str =
    "Brings out the shape of hills with smoother normals and a crisper edge between sun and shade. Per-pixel land shading only (UOD-Painterly); visible on slopes, not on flat ground.";
pub(super) const SOFT_LIGHTING: &str =
    "Adds soft light from the sky and lets sunlight reach further around slopes, lifting the shade. Per-pixel land shading only; strongest at night in UOD-Painterly.";
pub(super) const HIGHLIGHTS: &str =
    "Adds a shine where land reflects the sun and a thin light on the edges of slopes turned away from the camera. Per-pixel land shading only; both are faint at the UOD-Painterly values.";
pub(super) const KR_LAND_TEMPERATURE: &str =
    "Warms sunlit land and cools shaded land. KR shading with per-pixel land shading (UOD-Painterly); visible by day.";
pub(super) const KR_LAND_RELIEF_SHADOW: &str =
    "Darkens land at the foot of height steps and along cliff banks. KR shading with per-pixel land shading (UOD-Painterly); visible by day.";
pub(super) const KR_LAND_SHADOW_MOTTLE: &str =
    "Breaks the shade on land into soft blotches. KR shading with per-pixel land shading (UOD-Painterly). Faint on shaded slopes up to about 0.6, the UOD-Painterly value; past it the blotches darken and spread over sunlit land too, for a gloomy, cloud-shaded look at the maximum.";
pub(super) const KR_MATERIAL_CONTRAST: &str =
    "Makes grass, rock and other land textures slightly richer and more varied from tile to tile. Needs the KR visual profile (UOD-Painterly), not on water; strongest by day.";
pub(super) const KR_TRANSITION_GRIME: &str =
    "Dirties and darkens the edges where one kind of land meets another. Needs the KR visual profile (UOD-Painterly); clear by day, faint at night.";
pub(super) const LAND_TEXTURE_VARIATION: &str =
    "Hides the grid in which the KR and Enhanced terrain textures repeat, every few cells, on grass, dirt, sand, forest, snow, swamp, jungle, rock and cool lava. Each texture is drawn through overlapping hexagons two cells wide, each showing it moved by its own random offset, and turned by a random quarter turn where the texture has no direction; its blend between the two layers follows a larger grid of its own. Cobblestone, planks, bricks, tiles, furrows and cliffs keep their plain tiling. runtime_specs/land/LandTextureVariation.kdl lists the materials. Costs a few more texture reads per pixel, and fades out as you zoom out, where the repetition no longer shows. One setting for the style at every time of day. Off in every preset.";

pub(super) const NORMAL_MAPS: &str =
    "Gives land textures small bumps that catch the sun. Enhanced and KR snow use the bump map the client ships with it; every other land texture, Classic texmaps included, derives its bumps from its own light and dark texels. Works in every render style and land source: per-pixel land shading lights the bumps directly, Classic shading and the clients' per-vertex shading brighten and darken their own light by them. Water and other liquids stay flat. The derived bumps fade out as you zoom out. Off in every preset.";

// --- Standard and KR-style art ---

pub(super) const ART_SHADOW_SHAPE: &str =
    "Darkens the shaded side of items and buildings to give them volume. Clear in UOD-Painterly, too light to see in Classic; greyed out in KR-like, where Client Shadows replace it.";
pub(super) const ART_CONTACT_SHADOWS: &str =
    "Darkens items where they meet the ground. Needs Fake Normals on (UOD-Painterly); greyed out in KR-like, where Client Shadows replace it.";
pub(super) const ART_MOTTLE: &str =
    "Adds blotchy light and dark patches over items such as roofs and tree canopies, in every render style. Off in every preset; visible from about 1.0.";
pub(super) const ART_LIGHT_CATCH: &str =
    "Brightens the sunlit side of items and buildings. Clear in UOD-Painterly; too light to see in Classic.";
pub(super) const ART_ATMOSPHERE_TINT: &str =
    "Fades upright items toward the Atmosphere Tint colour (Advanced > Lighting Colors) for a sense of depth. Clear in UOD-Painterly; too light to see in Classic.";
pub(super) const KR_ART_TEMPERATURE: &str =
    "Warms the sunlit side of items and cools their shaded side. Needs Fake Normals on (UOD-Painterly); faint, mostly by day.";
pub(super) const ART_PROJECTED_SHADOWS: &str =
    "Casts a soft shadow from each item onto the ground. On in UOD-Painterly; greyed out in KR-like, where Client Shadows replace it.";
pub(super) const ART_FAKE_NORMALS: &str =
    "Shades items as if they had depth, guessing their shape from the sprite. On in UOD-Painterly; Contact Shadows and Temperature need it.";

// --- Advanced: effects and modes ---

pub(super) const VISUAL_PROFILE: &str =
    "Which look's tone curve and style-only effects are used: Neutral, EC or KR. The Render Style buttons set it; the KR effects need KR here.";
pub(super) const SHADING_MODE: &str =
    "How land is lit: per corner (Classic) or per pixel (Enhanced, KR). It decides which terrain settings act; the Render Style buttons set it.";
pub(super) const NORMALS: &str =
    "Bicubic uses the slope of the neighbouring tiles for rounder hills; Geometric shades each tile's own slope. Visible on slopes with per-pixel land shading; Classic always uses Geometric.";
pub(super) const GRUNGE_STRENGTH: &str =
    "How strong the dirt and wear texture is. Needs Grunge on.";
pub(super) const BENT_NORMALS: &str =
    "Evens out the light on land below higher ground, such as the foot of a slope or a dip. Acts in Classic and with per-pixel land shading; on in Classic and UOD-Painterly.";
pub(super) const GLOOM: &str =
    "Darkens and cools shaded land at sea level for a moodier look. Enhanced and KR shading with per-pixel land shading (on in UOD-Painterly); clearest at night. It and the Gloom section are greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const BLUR: &str =
    "Softens land textures slightly, in every render style, until zoomed far in. On in UOD-Painterly; land only.";

// --- Advanced: intensities ---

pub(super) const EXPOSURE: &str =
    "Brightens or darkens the whole frame before the tone curve. Needs Tonemap on; the presets raise it a little by day.";
pub(super) const SPECULAR: &str =
    "A shine where land reflects the sun. With the UOD-Painterly sun, behind the scene, it lays an even grey sheen over all flat land: faint at the UOD-Painterly values, clear by day from about 0.002. Per-pixel land shading only; it is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const FILL: &str =
    "Soft light from the sky added to all land, lifting the shade. Per-pixel land shading only; strongest at night in UOD-Painterly. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const SHARPNESS_FACTOR: &str =
    "How crisp the line between sunlit and shaded land gets. Acts only through Sharpness Mix; greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const SHARPNESS_MIX: &str =
    "How much of the crisper sun and shade line is used. Visible in UOD-Painterly at night and in caves; greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const BLUR_STRENGTH: &str =
    "How much of the blurred texture is mixed in. Needs Blur on.";
pub(super) const BLUR_RADIUS: &str =
    "How far the land blur reaches, in screen pixels; the shader never blurs less than half a pixel. Needs Blur on.";
pub(super) const RIM: &str =
    "A thin light on the edges of slopes turned away from the camera, at full strength in shade and a fifth of it in sunlight. Enhanced and KR per-pixel shading share it; Enhanced draws it white at a quarter of the strength, too faint to see even at the slider maximum. Faint at the UOD-Painterly values. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const DIFFUSE_WRAP: &str =
    "Lets sunlight reach further around slopes, softening the edge of the shade. KR shading only, greyed out unless KR shading has per-pixel land shading, since Enhanced, Classic and the per-vertex model never read it; Enhanced's per-pixel light is a plain Lambert.";

// --- Advanced: lighting colours ---

pub(super) const LIGHT_COLOR: &str =
    "Colour of the sunlight on land and on the lit side of items; values above 1 brighten. On land only with per-pixel land shading (UOD-Painterly, strongest at night); Classic land and the clients' per-corner land ignore it.";
pub(super) const ATMOSPHERE_TINT: &str =
    "Colour that upright items fade toward with Atmosphere Tint, and the gloom colour when Gloom Tint is black. Visible in UOD-Painterly.";
pub(super) const FILL_SKY: &str =
    "Colour and strength (alpha) of the sky light from Soft Lighting. Per-pixel land shading only; its colour is hard to see. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const FILL_GROUND: &str =
    "Colour and strength (alpha) of the soft light from below, which reaches only land facing down, such as cliff overhangs. Per-pixel land shading with Soft Lighting; barely visible even at full strength. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const RIM_COLOR: &str =
    "Colour of the rim light on land; values above 1 brighten it. KR shading only, greyed out unless KR shading has per-pixel land shading, since Enhanced, Classic and the per-vertex model never read it; Enhanced's rim is white. Faint at the UOD-Painterly rim: a tint along the edges of slopes turned away from the camera.";
pub(super) const RIM_POWER: &str =
    "How thin the rim light is: higher keeps it to land seen closer to edge-on, so less of each slope catches it. In UOD-Painterly it is clear from about 1 down to 0.1, the widest rim; Enhanced's rim stays too faint to see at any power. Per-pixel land shading with some Rim; greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";

// --- Advanced: colour grading ---

pub(super) const GRADE_STRENGTH: &str =
    "How much of the colour grading is applied; 0 is the same as Color Grading off.";
pub(super) const HEADROOM_RESERVE: &str =
    "How far below white the KR light on land is capped, so sunlit ground does not bleach. KR shading with per-pixel land shading, by day, with Enable Headroom Limit on; greyed out unless KR shading has per-pixel land shading, since Enhanced, Classic and the per-vertex model never read it; Enhanced caps at a fixed 0.9.";
pub(super) const FILL_CHROMA_TINT: &str =
    "How much the sky light's colour tints land in shade. Enhanced and KR per-pixel shading; very faint. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const HEADROOM_LIMIT: &str =
    "Caps the light on land so sunlit ground keeps its colour instead of washing out. Enhanced and KR shading with per-pixel land shading, by day; turning it off brightens daylit land a lot. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it.";
pub(super) const VIBRANCE: &str =
    "Boosts dull colours more than vivid ones. Needs Color Grading on.";
pub(super) const SATURATION: &str =
    "Strength of all colours; 1 leaves them unchanged. Needs Color Grading on.";
pub(super) const CHROMA_KNEE: &str =
    "Tones down only the vivid colours: a colour more saturated than the knee is pulled toward grey, keeping its brightness and hue, while duller colours stay as they are. 1 is off; KR-like uses it in place of a lower Saturation to bring its greens to the KR screenshots without greying stone and wood. Needs Color Grading on.";
pub(super) const CHROMA_KNEE_SLOPE: &str =
    "How much saturation above the Chroma Knee is kept: 1 keeps it all (no effect), 0 caps every colour at the knee. Needs Color Grading on and a Chroma Knee below 1.";
pub(super) const CONTRAST: &str =
    "Pushes darks and lights apart; 1 leaves them unchanged. Needs Color Grading on.";
pub(super) const SPLIT_TONE: &str =
    "Tints bright areas with Grade Warm and dark areas with Grade Cool; 0 is off. Needs Color Grading on.";
pub(super) const GRADE_WARM: &str =
    "Tint of bright areas in the split tone; only its hue matters, not its brightness. Needs Color Grading and Split Tone Strength; hard to see at the preset.";
pub(super) const GRADE_COOL: &str =
    "Tint of dark areas in the split tone; only its hue matters, not its brightness. Needs Color Grading and Split Tone Strength; clearly visible in UOD-Painterly.";

// --- Advanced: gloom ---

pub(super) const GLOOM_AMOUNT: &str =
    "How strong the gloom is. Needs Gloom on; switching Gloom on sets it to 0.2 when it is 0.";
pub(super) const GLOOM_FADE_HEIGHT: &str =
    "Height above which the gloom fades out, in world units (land spans about -12 to +12). The presets keep it to land at sea level; 0 covers land at every height.";
pub(super) const GLOOM_SHADOW_BIAS: &str =
    "Puts more of the gloom on land facing away from the sun. Visible in UOD-Painterly at night and in caves.";
pub(super) const GLOOM_TINT: &str =
    "Colour of the gloom, and in alpha how much colour it drains. Black uses the Atmosphere Tint; visible in UOD-Painterly at night.";

// --- Advanced: graphics ---

pub(super) const STATIC_LIGHT_DECALS: &str =
    "Strength of the glow drawn around light sources. Shows at night and in caves in Classic and UOD-Painterly, and grows as Brightness drops; greyed out in KR-like and Enhanced, which light the night through the client's light buffer.";
pub(super) const SHARPENING: &str =
    "Sharpens land textures, in every render style. 0 in every preset; even 0.5 changes most of the frame.";
pub(super) const KR_COLOR_WASH_STRENGTH: &str =
    "How strong the colour wash is. Needs KR Color Wash on and the KR visual profile.";
pub(super) const NORMAL_MAP_STRENGTH: &str =
    "How strong the land bumps are: how far a bump map tilts the surface, or how deep the bumps derived from a texture are. Needs Land Bump Maps on.";
pub(super) const KR_TRANSITION_GRIME_STRENGTH: &str =
    "How strong the grime on land borders is. Needs KR Transition Grime on and the KR visual profile.";
pub(super) const ART_PROJECTED_SHADOW_STRENGTH: &str =
    "How dark the item shadows are. Needs Projected Shadows on.";
pub(super) const ART_PROJECTED_SHADOW_LENGTH: &str =
    "How long the item shadows are. Needs Projected Shadows on.";
pub(super) const ART_PROJECTED_SHADOW_SOFTNESS: &str =
    "How soft the edge of the item shadows is. Needs Projected Shadows on; the difference is small.";

// --- Advanced: fog ---

pub(super) const FOG_COLOR: &str =
    "Colour of the haze, and in alpha how thick it can get (0 turns fog off). Needs Fog on.";
pub(super) const FOG_DISTANCE_DENSITY: &str =
    "How far the haze reaches in from the screen edges. Needs Fog on; with this and Height Density at 0 there is no fog at all.";
pub(super) const FOG_HEIGHT_DENSITY: &str =
    "Extra haze toward the bottom of the screen. Needs Fog on; too faint to see at the UOD-Painterly values.";
pub(super) const FOG_NOISE_SCALE: &str =
    "Size of the drifting cloud patches. Needs Fog on, some Distance or Height Density, and Noise Strength above 0.";
pub(super) const FOG_NOISE_STRENGTH: &str =
    "How visible the slowly drifting cloud patches are. Needs Fog on and some Distance or Height Density: the clouds only shape existing haze.";
pub(super) const FOG_NIGHT_TINT: &str =
    "A colour mixed into the haze, by the amount in alpha; the presets use more at night and in caves. Hard to see at the preset values.";
