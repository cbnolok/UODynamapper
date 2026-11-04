# Shader controls: scope and responsive layout

The main window shows every ordinary control at once, with no collapsed section.
The top row picks the preset: Render Style and Time of Day. Below it, sections
follow what each control acts on:

- Whole scene: Tonemap, Color Grading, Texture Smoothness and Fog on one row,
  then Brightness, KR Tonemap Brightness and Light/Static Interaction beside
  Grunge, KR Color Wash and its scale.
- Terrain and water: Water Animation, the land sliders, KR Transition Grime and
  Normal Maps. Water animation also moves static water in the art shaders, and
  the caption says so.
- Standard art and KR-style art: two columns of art sprite sliders.

Those last three sit in three columns from 900 points of content width, in two
from 600, and stack below that. A control with its own toggle and strength shares
one row, with its slider grayed while it is off. Controls the current style never
reads are grayed. The KR-style column stays usable in every render style, because
art shading does not depend on the land style or profile gates. That includes
[Fake Normals, which also work on Classic art](art_fake_normal_validation.md). The
helpers return their changed flags to the existing dirty and preset-save path.

Advanced Effects separates whole-scene processing from water surfaces.
Advanced Graphics Settings separates whole-scene, terrain and object controls.
Mode and normal selection use separate rows to keep the side window narrow.

The advanced window's initial position comes from the actual main window's
right edge plus 12 points. An explicit 360-point default width is necessary:
egui otherwise initially constrains its larger default **area** before measuring
the narrower window, permanently shifting the starting position left. Merely
changing `default_pos` leaves an overlap. Both windows remain movable/resizable;
no per-frame position override is used. The main window opens at 960 by 580
points, enough to show all sections and the buttons below them without scrolling.

Native 1280x800 Desktop-4 validation uses all three styles, real resize drags,
both column/stacked layouts, advanced Effects/Graphics, and clicking the moved
fake-normal and water toggles. Closing F3 saves both enabled values in the
Enhanced morning preset, and subsequent interaction restores them. Four
existing preset/control tests and the normal build pass. Captures and input
records use `shader-layout-accepted-` in the temporary renderer workspace.
These checks establish UI behavior; they do not close the separate visual
effect/decal or non-KR wall-artifact investigations. They predate the section
layout above, which has only been captured, in all three styles at UI scale 1 and
1.15; its resize breakpoints were not dragged natively.
