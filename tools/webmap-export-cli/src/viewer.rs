//! The static viewer: `index.html` and the vendored Leaflet 1.9.4 (`assets/leaflet-1.9.4/SOURCE.txt`).

use color_eyre::eyre;

use crate::formats::Plan;
use crate::geometry::{Layout, TILE_HALF_PX};
use crate::storage::{OutputDir, write_atomic};

/// Zoom levels above the base level, drawn by the browser with pixelated scaling.
pub const OVERZOOM_LEVELS: u32 = 2;
pub const INDEX_FILE: &str = "index.html";
pub const LEAFLET_DIR: &str = "leaflet";

const LEAFLET_FILES: [(&str, &[u8]); 3] = [
    ("leaflet.js", include_bytes!("../assets/leaflet-1.9.4/leaflet.js")),
    ("leaflet.css", include_bytes!("../assets/leaflet-1.9.4/leaflet.css")),
    ("LICENSE", include_bytes!("../assets/leaflet-1.9.4/LICENSE")),
];

const TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>@TITLE@</title>
<link rel="stylesheet" href="leaflet/leaflet.css">
<style>
html, body, #map { height: 100%; margin: 0; background: #000; }
.leaflet-container { background: #000; }
.leaflet-tile { image-rendering: pixelated; }
.coords { background: rgba(0, 0, 0, 0.7); color: #eee; font: 13px/1.4 monospace; padding: 2px 6px; }
</style>
</head>
<body>
<div id="map"></div>
<script src="leaflet/leaflet.js"></script>
<script>
"use strict";
// Base-zoom pixels: the top vertex of tile (x, y) at height z is at
// ((x - y + height) * halfTile, (x + y) * halfTile - 4 * z + zTop).
const facet = {
  id: @FACET@, width: @WIDTH@, height: @HEIGHT@, zTop: @Z_TOP@, halfTile: @HALF_TILE@,
  baseZoom: @BASE_ZOOM@, maxZoom: @MAX_ZOOM@, tileSize: @TILE_SIZE@,
  bboxWidth: @BBOX_WIDTH@, bboxHeight: @BBOX_HEIGHT@, view: [@VIEW@],
};
const map = L.map("map", { crs: L.CRS.Simple, minZoom: 0, maxZoom: facet.maxZoom, zoomSnap: 1 });
const toLatLng = (px, py) => map.unproject([px, py], facet.baseZoom);
const bounds = L.latLngBounds(toLatLng(0, facet.bboxHeight), toLatLng(facet.bboxWidth, 0));
L.tileLayer("tiles/{z}/{x}/{y}.webp", {
  tileSize: facet.tileSize, minZoom: 0, maxZoom: facet.maxZoom, maxNativeZoom: facet.baseZoom,
  noWrap: true, bounds: bounds, errorTileUrl: "tiles/empty.webp",
  attribution: "Ultima Online &copy; Electronic Arts",
}).addTo(map);
const [left, top, right, bottom] = facet.view;
map.fitBounds(L.latLngBounds(toLatLng(left, bottom), toLatLng(right, top)));
const Readout = L.Control.extend({
  onAdd() { this.div = L.DomUtil.create("div", "coords"); this.div.textContent = `facet ${facet.id}`; return this.div; },
});
const readout = new Readout({ position: "bottomleft" }).addTo(map);
// The ground tile under the cursor, at z = 0 (raised terrain reads about z / 11 tiles off).
map.on("mousemove", (event) => {
  const p = map.project(event.latlng, facet.baseZoom);
  const u2 = Math.floor(p.x) - facet.height * facet.halfTile;
  const v2 = Math.floor(p.y) - facet.zTop;
  const x = Math.floor((u2 + v2) / (2 * facet.halfTile));
  const y = Math.floor((v2 - u2) / (2 * facet.halfTile));
  const inside = x >= 0 && y >= 0 && x < facet.width && y < facet.height;
  readout.div.textContent = inside ? `facet ${facet.id}  x ${x}  y ${y}` : `facet ${facet.id}`;
});
</script>
</body>
</html>
"#;

/// The viewer page for a plan.
pub fn index_html(plan: &Plan, layout: &Layout) -> String {
    let view = match plan.region {
        Some([x0, y0, x1, y1]) => layout.region_rect(x0, y0, x1, y1),
        None => crate::geometry::PixelRect {
            left: 0,
            top: 0,
            width: layout.bbox_width_px() as i64,
            height: layout.bbox_height_px() as i64,
        },
    };
    let replacements = [
        ("@TITLE@", format!("Facet {} web map", plan.facet)),
        ("@FACET@", plan.facet.to_string()),
        ("@WIDTH@", plan.map_width_tiles.to_string()),
        ("@HEIGHT@", plan.map_height_tiles.to_string()),
        ("@Z_TOP@", layout.z_top_px.to_string()),
        ("@HALF_TILE@", TILE_HALF_PX.to_string()),
        ("@BASE_ZOOM@", layout.base_level().to_string()),
        ("@MAX_ZOOM@", (layout.base_level() + OVERZOOM_LEVELS).to_string()),
        ("@TILE_SIZE@", layout.grid.tile_size_px.to_string()),
        ("@BBOX_WIDTH@", layout.bbox_width_px().to_string()),
        ("@BBOX_HEIGHT@", layout.bbox_height_px().to_string()),
        ("@VIEW@", format!("{}, {}, {}, {}", view.left, view.top, view.left + view.width, view.top + view.height)),
    ];
    replacements.iter().fold(TEMPLATE.to_string(), |html, (key, value)| html.replace(key, value))
}

/// Writes `index.html` and the Leaflet files.
pub fn write_viewer(out: &OutputDir, plan: &Plan, layout: &Layout) -> eyre::Result<()> {
    for (name, data) in LEAFLET_FILES {
        write_atomic(&out.root.join(LEAFLET_DIR).join(name), data)?;
    }
    write_atomic(&out.root.join(INDEX_FILE), index_html(plan, layout).as_bytes())
}
