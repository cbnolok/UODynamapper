# Dynamapper showcase

[Quick start](../README.md#quick-start) · [Tools showcase](../tools/README.md)

## Britain in four render styles

The afternoon captures use the same position and camera on Trammel. Art and land are listed separately because changing a render preset leaves both source settings in place.

| Classic | Enhanced |
| --- | --- |
| ![Britain square in Classic style](images/showcase/britain-classic.webp) | ![Britain square in Enhanced style](images/showcase/britain-enhanced.webp) |
| Classic art and terrain | Enhanced art and terrain |

| KR-like | UOD-Painterly |
| --- | --- |
| ![Britain square in KR-like style](images/showcase/britain-kr-like.webp) | ![Britain square in UOD-Painterly style](images/showcase/britain-uod-painterly.webp) |
| Enhanced art, Kingdom Reborn terrain | Enhanced art and terrain |

![Britain square at night in KR-like style](images/showcase/kr-like-britain-night.webp)

KR-like night lighting in Britain. [Render style rules](dev_wiki/rendering/STYLE_RULES.md) · [Lighting and shadow research](dev_wiki/rendering/CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md)

## Mixing Classic art with Enhanced and KR terrain

Each pair keeps the style, terrain source, and camera fixed. Compare the statue, stairs, and tree to see what changes when you select Classic art.

| Enhanced art and Enhanced terrain | Classic art and Enhanced terrain |
| --- | --- |
| ![Enhanced art over Enhanced terrain in Britain](images/showcase/britain-enhanced.webp) | ![Classic art over Enhanced terrain in Britain](images/showcase/britain-classic-art-enhanced-land.webp) |

| Enhanced art and Kingdom Reborn terrain | Classic art and Kingdom Reborn terrain |
| --- | --- |
| ![Enhanced art over Kingdom Reborn terrain in Britain](images/showcase/britain-kr-like.webp) | ![Classic art over Kingdom Reborn terrain in Britain](images/showcase/britain-classic-art-kr-land.webp) |
| ![Enhanced art over Kingdom Reborn terrain in Minoc](images/showcase/minoc-town-enhanced-art-kr-land.webp) | ![Classic art over Kingdom Reborn terrain in Minoc](images/showcase/minoc-town-classic-art-kr-land.webp) |

Open **F2 → Graphics** to select **Art Texture Source** and **Land Texture Source**. World objects that continue the ground, such as grass and liquids, can take their material and art from the selected terrain source. [Art and terrain source rules](dev_wiki/rendering/STYLE_RULES.md#sources-are-not-part-of-the-style)

## Local lighting after dark

| Lantern by the Minoc mine | Braziers in the hedge maze |
| --- | --- |
| ![A lantern lights the path and the base of a wall beneath a dark roof](images/showcase/light-lantern.webp) | ![Braziers light the columns and paving beside a hedge](images/showcase/light-brazier.webp) |
| 2507, 560 | 1072, 2269 |

| Lava channel | Lit windows |
| --- | --- |
| ![Glowing lava casts warm light across a rocky trench](images/showcase/light-lava.webp) | ![Window lights warm the outer walls of a stone building and cast light onto its paving](images/showcase/light-window.webp) |
| 4622, 3560 | 4475, 1167 |

*Felucca in KR-like night lighting, with Enhanced art, Kingdom Reborn terrain, and optional local lighting enabled.*

The mine lantern lights the path and wall under the eave. The braziers pool light across the columns and paving, while the lava lights the rocks along its banks. In KR-like and Enhanced styles, **F3 → Local Lights** lets you choose between light sprites and `light.mul` and adjust the selected source's intensity. [Night lighting and light cover](dev_wiki/rendering/CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md)

## RIFE frame interpolation

![Enhanced daemon walking in four panels with original frames, crossfade, RIFE ×2, and RIFE ×4](images/showcase/anim-rife-daemon-loop.webp)

This five-second comparison shows an Enhanced daemon walking east across Trammel. All four Dynamapper recordings use the same route, zoom, client timing, and ground speed. **Original frames** holds each sprite pose until the next one. **Crossfade** blends neighboring poses. **RIFE ×2** and **RIFE ×4** generate one and three intermediate poses per frame pair. Watch the wings and feet during each step.

![Enhanced Stygian dragon sprite showing two source frames, a crossfade and a RIFE-generated midpoint](images/showcase/anim-rife-stygian.webp)

The two source frames are on the outside. In the middle, the crossfade blends the two wing outlines while RIFE estimates an intermediate pose. Dynamapper runs RIFE on the GPU and generates one or three in-between frames per pair in the ×2 and ×4 modes. The strip is from the offline RIFE comparison used during development.

![Classic wyrm sprite showing source frames, a crossfade and a RIFE midpoint](images/showcase/anim-rife-classic.webp)

The same comparison on a Classic wyrm. Look at the near wing and feet.

See the [animation development notes](agent_notes/mobile_animation_motion.md) for the GPU inference and playback implementation.

## Animation timing and measured strides

![Enhanced human on horseback with Client timing and Match the ground](images/showcase/anim-stride-horse-loop.webp)

An Enhanced rider and horse take the same walk across Trammel. **Client timing** plays the original animation cadence. **Match the ground** paces both animations using the mount's measured stride. Watch the hooves against the ground as it scrolls by.

![Enhanced human walking with Client timing and Match the ground](images/showcase/anim-stride-human-loop.webp)

The same comparison on foot. The ground moves at the same speed in both panels. Follow the Enhanced human's planted foot to see the change in cadence.

[![Enhanced wyrm walk with client timing on the left and measured stride on the right](images/showcase/anim-stride-wyrm.webp)](images/showcase/anim-stride-wyrm-loop.webp)

Open the [animated side-by-side comparison](images/showcase/anim-stride-wyrm-loop.webp) and follow the feet against the paving. Both panels move over the ground at the same speed. The Enhanced wyrm uses a 1.33-second cycle at client timing and a 0.83-second cycle with the measured stride in this development comparison. The renderer uses a matching stride measurement from the animation package when available.

## Sprite upscaling

![UOCF Editor previewing an original Classic animation frame and its 2× upscaled result](images/showcase/anim-upscale-editor.webp)

The graphical editor previews upscale passes on animation frames. Image packers support ordered upscale profiles for Classic and Enhanced animation frames, gumps, terrain textures and art. See the [upscale profile guide](../tools/udd-conv-cli/README.md#upscale-profiles).

![Enhanced horse sprite enlarged with nearest-neighbor, bicubic and SPAN](images/showcase/anim-upscale-span.webp)

An offline 4× comparison on an Enhanced horse frame. Dynamapper also offers **SPAN 2×** in its graphics options. It runs on the GPU for Enhanced animations at close zoom, including RIFE in-betweens. [Runtime animation and upscaling notes](agent_notes/mobile_animation_motion.md#span-2-in-game-mobile_upscale-2026-11-21)

## Cave entrances

![A cave arch embedded in the hillside north of Britain](images/showcase/world-cave-outside.webp)

North of Britain, the cave arch meets the hillside. From inside, the entrance and the cave floor remain visible, and the outside can be dimmed or drawn in black.

| Dimmed surroundings | Black surroundings |
| --- | --- |
| ![Cave interior with the terrain around the entrance dimmed](images/showcase/world-cave-dimmed.webp) | ![Same interior with the cave isolated against black](images/showcase/world-cave-black.webp) |

See the [cave rendering rules](dev_wiki/rendering/STYLE_RULES.md#caves-every-style).

## Floor and room views

| Roof and upper floors visible | Current building cut away | Cutaway across the view |
| --- | --- | --- |
| ![Britain riverside building with the roof visible](images/showcase/world-floor-off.webp) | ![The room is visible and the neighboring roof remains](images/showcase/world-floor-building.webp) | ![The room is visible and surrounding structures are cut away as well](images/showcase/world-floor-wide.webp) |

In **F2 → Graphics**, enable **Show Only the Player's Floor**. **Only the Player's Building** keeps the cut to the current building so surrounding roofs and their lights can remain. All three captures use the same camera and player position.

## Zoom range

| Close enough to inspect the paving | Wide enough to read the town layout |
| --- | --- |
| ![Close-up of the Britain square planter, stairs, and individual pavers](images/showcase/world-zoom-close.webp) | ![Wide view of Britain with streets, waterways, the castle, and docks](images/showcase/world-zoom-wide.webp) |

Both captures are centered on the same point in Britain. The wide view covers 32 times as much world width as the close view.

At scale 20 and wider, ordinary statics use packed radar colors. River and shoreline water keep their liquid materials, with foam and transparent edges filtered as the view zooms out.

![Wide KR-like view of Minoc with textured river water and shoreline foam](images/showcase/world-zoom-minoc-overview.webp)

*Minoc at scale 20 with Enhanced art and Kingdom Reborn terrain.*

![Maximum-zoom live view of the Trammel mainland, its cities, and offshore islands](images/showcase/world-zoom-trammel-overview.webp)

*Trammel at scale 50 in KR-like style. This live view covers more than half of the mainland bounds.*

[View the full Trammel facet as a radar export](images/showcase/world-trammel-radar.webp). The converter builds this top-down image from the map and its statics.

## Paperdolls

![Two Enhanced paperdolls showing the same robe and lantern, rendered with 3D-style frames on the left and gumps on the right](images/showcase/ui-paperdolls.webp)

The same outfit in both Enhanced paperdoll modes. The 3D-style frames show the lantern's larger glow. Pack the gump and paperdoll data and set `load_gumps = true` in `assets/settings/runtime_assets.toml` before launching. Press **Ctrl+U** and select **Paperdoll** to preview equipment and hues. The paperdoll's context menu lets you override garment coverage for a robe.

## Live settings

![Land Shader Controls showing the KR-like night preset over Britain](images/showcase/ui-settings.webp)

Open Land Shader Controls with **F3** to switch styles and times of day, and tune lighting and materials for the active preset.

![Graphics options with separate art and terrain choices, animation controls, and building and cave visibility](images/showcase/ui-graphics-options.webp)

Open Options with **F2** to select art and terrain independently and adjust animation, camera, and visibility settings.
