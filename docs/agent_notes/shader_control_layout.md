# Shader controls: scope and responsive layout

Basic Controls separates Terrain, Objects, Water surfaces and Everything.
Water animation affects both map water and static water in the art shaders;
placing it under terrain alone or whole-scene processing hides that scope.
Objects uses Standard art and KR-style art columns when its available content
width is at least 560 egui points. Narrower windows stack the same controls.
The KR-style column remains usable in every render style: art shading is
independent of the land style/profile gates. Its caption states this explicitly.
The two helpers return their combined changed flags to the existing dirty and
preset-save path. No preset values, effect gates or shader behavior changed.

Advanced Effects separates whole-scene processing from water surfaces.
Advanced Graphics Settings separates whole-scene, terrain and object controls.
Mode and normal selection use separate rows to keep the side window narrow.

The advanced window's initial position comes from the actual main window's
right edge plus 12 points. An explicit 360-point default width is necessary:
egui otherwise initially constrains its larger default **area** before measuring
the narrower window, permanently shifting the starting position left. Merely
changing `default_pos` leaves an overlap. Both windows remain movable/resizable;
no per-frame position override is used. Main width defaults to 640 points.

Native 1280x800 Desktop-4 validation uses all three styles, real resize drags,
both column/stacked layouts, advanced Effects/Graphics, and clicking the moved
fake-normal and water toggles. Closing F3 saves both enabled values in the
Enhanced morning preset, and subsequent interaction restores them. Four
existing preset/control tests and the normal build pass. Captures and input
records use `shader-layout-accepted-` in the temporary renderer workspace.
These checks establish UI behavior; they do not close the separate visual
effect/decal or non-KR wall-artifact investigations.
