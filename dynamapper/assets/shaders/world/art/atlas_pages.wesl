// ============================================================================
// art::atlas_pages — The two atlases of the art pages (`ArtPageAtlasSet` in art.rs): the Classic pages, and the
// Enhanced ones at their own page size and format. An instance's layer carries which of them its page lies in.
// ============================================================================

import package::shaders::world::client_filter::client_filtered_atlas_rect;

// Set on an instance's layer when its page lies in the Enhanced atlas (`ART_LAYER_ENHANCED_ATLAS_BIT` in art.rs).
const ART_LAYER_ENHANCED_ATLAS_BIT: u32 = 0x80000000u;

// A page: its layer in its atlas, and whether that atlas is the Enhanced one.
struct ArtPage {
    layer: i32,
    enhanced: bool,
}

// The instance's page as the vertex stage hands it on, in a varying: its layer, then 1 for the Enhanced atlas. The
// layer leaves the atlas bit out, which an f32 could not carry with it.
fn art_page_varying(encoded_layer: u32) -> vec2<f32> {
    let enhanced = (encoded_layer & ART_LAYER_ENHANCED_ATLAS_BIT) != 0u;
    return vec2<f32>(f32(encoded_layer & ~ART_LAYER_ENHANCED_ATLAS_BIT), select(0.0, 1.0, enhanced));
}

// The page `art_page_varying` handed on.
fn art_page(handed_on: vec2<f32>) -> ArtPage {
    return ArtPage(i32(handed_on.x), handed_on.y > 0.5);
}

// One tap of the page. The atlases hold a single mip level, so the level-0 tap is the implicit-derivative one, and
// it may sit in the branch on the page's atlas.
fn art_page_sample(classic_atlas: texture_2d_array<f32>, enhanced_atlas: texture_2d_array<f32>,
    atlas_sampler: sampler, page: ArtPage, uv: vec2<f32>) -> vec4<f32> {
    if (page.enhanced) {
        return textureSampleLevel(enhanced_atlas, atlas_sampler, uv, page.layer, 0.0);
    }
    return textureSampleLevel(classic_atlas, atlas_sampler, uv, page.layer, 0.0);
}

// The size, in texels, of the page's atlas.
fn art_page_atlas_size(classic_atlas: texture_2d_array<f32>, enhanced_atlas: texture_2d_array<f32>, page: ArtPage)
    -> vec2<f32> {
    if (page.enhanced) {
        return vec2<f32>(textureDimensions(enhanced_atlas).xy);
    }
    return vec2<f32>(textureDimensions(classic_atlas).xy);
}

// The client's filter on a sprite's rectangle of the page (client_filter.wesl).
fn art_page_client_filtered(classic_atlas: texture_2d_array<f32>, enhanced_atlas: texture_2d_array<f32>,
    page: ArtPage, uv: vec2<f32>, uv_min: vec2<f32>, uv_max: vec2<f32>, wrap: bool, uv_dx: vec2<f32>,
    uv_dy: vec2<f32>, client_view_scale: f32) -> vec4<f32> {
    if (page.enhanced) {
        return client_filtered_atlas_rect(enhanced_atlas, page.layer, uv, uv_min, uv_max, wrap, uv_dx, uv_dy,
            client_view_scale);
    }
    return client_filtered_atlas_rect(classic_atlas, page.layer, uv, uv_min, uv_max, wrap, uv_dx, uv_dy,
        client_view_scale);
}
