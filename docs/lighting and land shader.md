
## Big-picture pipeline

At each fragment (or per vertex if Gouraud) we conceptually evaluate:

```
final_rgb =
  diffuse(N, L)                 // Lambert
+ rim(N, V)                     // view-grazing brightness
+ fill(N)                       // hemispherical secondary light
+ specular(N, L, V)             // subtle gloss
→ color grading                 // warm/cool, painterly bias
→ tonemap                       // HDR → displayable SDR
```

* **N** = surface normal (geometric, bicubic, or bent).
* **L** = light direction (normalized; “sun”).
* **V** = view direction (eye → fragment).
* Optional **cloud/fog mask** multiplies the lit color after lighting but before grading/tonemap for KR mood.

---

## 1) Geometry, grids, and why 8×8 → 9×9 → 12×12/13×13

### 1.1 What we draw vs what we need

* **Core tiles we render:** **8×8** per chunk (the actual quads).
* **Vertex grid we build:** **9×9** (one more in each axis) so every quad has 4 distinct corners and so we can compute edge-safe vertex normals/displacements without falling off the mesh.
* **Shader “tile data” we upload:** **≥ 11×11** to support smooth per-fragment reconstruction (bicubic) and optional ambient/bent-normal sampling near borders.

### 1.2 Why 9×9 vertices for an 8×8 core

* Each of the 8×8 quads needs a fourth corner ⇒ you need **9 positions** per axis.
* When we do any world-space displacement or compute a geometric normal at vertices using central differences, you want neighbors on both sides; that extra ring avoids seams.

### 1.3 Why more than 9×9 for the shader

Per-fragment **bicubic** evaluation of a heightfield at a point inside tile `[i, i+1]` needs a **4×4** neighborhood in each axis: indices `{i-1, i, i+1, i+2}` × `{j-1, j, j+1, j+2}`.
Near the **interior**, those are inside the 9×9 grid. Near the **edges**, you need samples **outside** the 9×9.

Minimum for *just* bicubic everywhere in the 8×8 interior:

* 9 (interior node grid) + 1 extra node on **each** side = **11×11**.

However, we also want:

* **Edge-safe filters** (e.g., slope, curvature),
* **Bent-normal / horizon probes** that step outwards by 1–2 nodes in a few directions,
* Consistency across chunk boundaries **without clamping**.

That motivates **larger margins**:

* 9 + 2×**2** = **13×13** (margin radius = 2 nodes) → robust for bicubic **and** small horizon/bent-normal kernels.
* 9 + 2×**1** = **11×11** (margin radius = 1 node) → just enough for bicubic, but tight for horizon sampling.
* **12×12** is an awkward in-between (9 + 3), implying an average margin of **1.5**—easy to run out on one side when kernels point outward.

**Conclusion:** If you plan **only bicubic** and no horizon/bent normals, **11×11 is sufficient**.
If you want **bent normals / tiny AO kernels** (our KR preset), **13×13 is the right choice**. The extra cost is tiny (see §1.5).

### 1.4 Why we sometimes say “12×12”

Some pipelines store **tile-center** attributes (one per tile), not node heights. An 8×8 core plus a 2-tile border becomes **12×12** “tiles”. But to do bicubic correctly you want **node** samples. In our implementation the uniform holds **per-cell attributes** including the height used for reconstruction; we standardized on **13×13** so we’re never forced to clamp or silently change kernels at borders.

### 1.5 Memory and cost check

`TileUniform` is **16 bytes** (std140-safe: `f32 + u32 + u32 + u32`).

* **12×12:** 144 tiles → **2304 B**.
* **13×13:** 169 tiles → **2704 B**.
  Difference: **400 B per chunk**—negligible versus the 64 KB UBO soft limit and well within Bevy/encase expectations.

**Recommendation:** Use **13×13**. Quality ↑, robustness ↑, cost \~0.

---

## 2) Normals: geometric, bicubic, bent

### 2.1 Geometric normals

* Derived from the **flat tile geometry** (piecewise planar).
* **Look:** stepped/faceted, faithful to classic 2D.
* **Pros:** cheap, stable.
* **Cons:** highlights/rim “pop” at tile edges; seams emphasize the grid.

### 2.2 Bicubic normals

* Reconstruct a **smooth heightfield** from the grid; differentiate to get `∂h/∂x, ∂h/∂y` and build a continuous `N`.
* **Look:** smooth, cohesive terrain; seams disappear.
* **Pros:** modern feel, great with per-fragment lighting.
* **Cons:** needs neighbor data (→ 13×13); slightly costlier.

### 2.3 Bent normals

* A **directionally biased** normal that tilts toward the most “open sky” direction (lower occlusion).
* We approximate by sampling a handful of **short horizon rays** along the heightfield (8–12 directions, 1–3 taps each).
* **Use:** feed bent `N` to **ambient/fill/rim** terms (not necessarily the core Lambert) to get soft, painterly atmospheric depth.
* **Note:** Needs extra margin; 13×13 is a comfortable minimum.

---

## 3) Lighting models vs shading methods

### 3.1 Lighting models (what you compute)

* **Lambert diffuse:** `max(dot(N, L), 0)` — matte base; energy-conserving.
* **Specular:**

  * **Phong:** `R = reflect(-L, N); spec = pow(max(dot(R, V), 0), shininess)`
  * **Blinn-Phong:** `H = normalize(L + V); spec = pow(max(dot(N, H), 0), shininess)`
    We use **Blinn-Phong** for efficiency and smoother lobes; keep intensity low for non-plastic KR.
* **Rim light:** `rim = pow(1 - dot(N, V), rim_power)` — view-grazing glow; painterly.
* **Secondary fill (hemisphere):** `k = 0.5 * dot(N, up) + 0.5; ambient = lerp(ground, sky, k)` — orientation-aware ambient.

### 3.2 Shading methods (where you compute)

* **Gouraud shading (per-vertex):** compute lighting at vertices and **interpolate**.

  * ✅ Cheap; matches classic look.
  * ❌ Rim/specular are **view-dependent** and local; interpolation **smears or kills** them.
  * Diffuse **is** affected by **light direction** (it’s Lambert at vertices), so changing sun angles still changes shading—just coarsely.
* **Per-fragment shading:** compute lighting **per pixel** using the interpolated normal or re-derived normal.

  * ✅ Accurate rim/specular; works with bicubic/bent normals.
  * ❌ Pricier; can shimmer if normals are noisy (we keep kernels stable).

> **Terminology note.** “**Phong shading**” historically means **per-fragment normal interpolation** with the **Phong reflection model** (Lambert + Phong spec). In modern usage people sometimes say “Phong” loosely to mean “per-fragment lighting.” We’ll say **per-fragment** for clarity and specify the reflection model (Lambert + Blinn-Phong, etc.).

---

## 4) Secondary fill (hemisphere light), in depth

* Real landscapes aren’t lit only by a single sun. Skydome and ground bounce create **broad, directionally dependent ambient**.
* Our hemisphere model is a **physically-inspired stylization**:

  * **Up-facing** surfaces blend toward **sky** (cool, low saturation).
  * **Down-facing** surfaces blend toward **ground** (warm, earthy).
* **Why KR likes it:** It avoids dead blacks, restores local color in shadow, and anchors objects to the world without flat ambient gray.
* **Controls:** `fill_strength`, `sky_color`, `ground_color`. We usually modulate the strength and let **grading** provide the final hue balance (warm/cool push).
* **Bent normals:** For fill, you may use `N_bent` instead of geometric/bicubic `N` to bias ambient toward open directions—this is the “free GI feel.”

---

## 5) HDR, tonemapping, and why 8-bit textures don’t make us LDR

### 5.1 Where HDR comes from (even with RGBA8 inputs)

Inputs are LDR, but **lighting math is additive/multiplicative** and easily exceeds 1.0:

* Rim (+), specular (+), fill (+), diffuse shaping/contrast (×), grading (±)
* Example: `base 0.8 × (diff 0.9 + rim 0.4) + spec 0.2 ≈ 1.36` (> 1).

Once any channel is > 1, you’re in **HDR**. The framebuffer or the logical color you carry needs compressing before display.

### 5.2 Why clamping is not acceptable

* `min(color, 1.0)` **clips** highlights → chalky whites, banding, and you lose the painterly roll-off that KR relies on.

### 5.3 Tonemapping choices

* **Reinhard:** `c_out = c_in / (1 + c_in)` — **soft shoulder**, ideal for painterly; simple and stable.
* **Filmic/Hable/ACES:** keep midtones/skin better; can be more contrasty.
* We prefer **Reinhard** (or gentle filmic) to preserve subtle highlight gradations from rim/spec/fill.

> Tonemapping isn’t for textures; it’s for the **lit result**. It’s essential when enabling KR features.

---

## 6) Feature compatibilities (and anti-patterns)

| Feature                    | Requires                     | Incompatible / Weak With         | Why                                              |
| -------------------------- | ---------------------------- | -------------------------------- | ------------------------------------------------ |
| **Gouraud**                | —                            | Rim/Spec (view-dependent)        | Interpolation smears/cancels highlights.         |
| **Per-fragment**           | Normal interpolation         | —                                | Needed for KR detail.                            |
| **Rim**                    | Per-fragment + smooth N      | Gouraud; geometric N (noisy)     | Needs stable per-pixel N; otherwise flicker.     |
| **Specular (Blinn-Phong)** | Per-fragment + smooth N      | Gouraud; geometric N             | Highlights pop on stepped normals.               |
| **Bicubic normals**        | 11×11+ data                  | Gouraud (benefit muted)          | Gouraud averages away per-pixel detail.          |
| **Bent normals**           | 13×13 data (margin=2)        | Geometric N (conceptually wrong) | Needs smooth field and space to probe.           |
| **Secondary fill**         | Any shading; better per-frag | —                                | Works everywhere; looks best with smooth/bent N. |
| **Grading**                | —                            | —                                | Style control; keep subtle in Classic.           |
| **Tonemap**                | HDR pipeline                 | —                                | Required with KR stack.                          |

**Rules of thumb:**

* **Classic:** Geometric + **Gouraud diffuse only**.
* **Enhanced Classic:** Bicubic + **per-fragment diffuse only** (+ subtle fill).
* **KR:** Bicubic/Bent + **full per-fragment stack** (rim/spec/fill) + grading + tonemap + fog/cloud.

---

## 7) 12×12 vs 13×13: does 13×13 really help?

### 7.1 What 12×12 gets you

* If your kernels never step beyond **±1** node from the 9×9 interior, **12×12 is almost enough** in one axis and **barely short** in the other (since 12 = 9 + 3 → average margin 1.5, but margins must be integers per side).
* In practice, at **some edges** you’ll need to clamp or switch to a smaller kernel. That produces **subtle lighting shifts at borders** (you’ll see it under rim/spec or strong fill).

### 7.2 What 13×13 guarantees

* Clean **margin radius = 2** around the entire 9×9 node grid.
* Bicubic everywhere in the 8×8 interior with **no clamping**.
* Small **bent-normal** (e.g., 8 directions × 2 taps) fits safely—no border condition changes.
* Enables **consistent edge-blend** logic between chunks.

### 7.3 Visual impact

* On *flat* terrain with **diffuse only**, 12×12 vs 13×13 is subtle.
* Once you enable **rim/spec/fill** (KR), edge-cases get amplified: highlight contours and ambient gradients make any border inconsistency obvious.
* 13×13 eliminates those hiccups.

### 7.4 Cost delta

* **+25 tiles** worth of uniforms (400 bytes) and a few extra CPU writes per chunk. GPU side unchanged.
* This is an easy quality win.

**Answer:** Yes—**13×13 provides a meaningful quality and robustness benefit** in our KR target, while the cost is negligible.

---

## 8) Edge blending and seams

Even with extra margins, cross-chunk seams can show if adjacent chunks reconstruct normals from slightly different neighborhoods. We mitigate by:

1. **Consistent expanded data**: 13×13 derived from the same world tiles that the neighbor will see.
2. **Edge blend factor** near borders (0 in the core → 1 at the outer ring) to mix:

   * geometric ↔ bicubic normals, or
   * vertex-computed world normal ↔ fragment-smooth normal.
     This avoids a hard switch in normal basis exactly on the border.
3. **Directional light is CPU-normalized** so dot products don’t vary in scale between chunks.

---

## 9) Diffuse shaping (“minimal grading” vs painterly shaping)

Besides tonemapping and color grading, we apply a **diffuse shaping** curve to tune the roll-off of Lambert:

```
lambert_shaped = mix(lambert, lambert^γ, mix_factor)
```

* With small `γ > 1` and low `mix_factor`, this is **“minimal grading”**: it gently boosts midtones and compresses high lights without bending color.
* In KR mode we can push shaping more aggressively **before tonemap** to get that “painted” soft roll-off.

Think of this as **luma-space pre-contrast** just on diffuse, not a global grade.

---

## 10) Style presets (toggle matrix)

### 10.1 Original / Classic 2D

* **Normals:** Geometric
* **Shading:** **Gouraud** (Lambert only)
* **Fill/Rim/Spec:** Off
* **Fog/Cloud:** Off
* **Grading:** Off (or trivial gamma)
* **Tonemap:** Off (stay LDR)
* **Goal:** faithful stepped look

### 10.2 Enhanced Classic 2D

* **Normals:** Bicubic
* **Shading:** **Per-fragment** Lambert
* **Fill:** Weak hemisphere (subtle)
* **Rim/Spec:** Off
* **Fog/Cloud:** Very weak (optional)
* **Grading:** Minimal (gentle warm/cool)
* **Tonemap:** On (mild; prevents clipping)
* **Goal:** “Remastered” cohesion; still classic

### 10.3 KR-like Painterly

* **Normals:** Bicubic + **Bent** for ambient terms
* **Shading:** **Per-fragment** Lambert + **Rim** + **Blinn-Phong Spec** (low, wide)
* **Fill:** Stronger hemisphere (sky/ground)
* **Fog/Cloud:** On (slow, low-freq, multiplicative)
* **Grading:** Painterly warm/cool bias
* **Tonemap:** **Reinhard / gentle filmic**
* **Goal:** soft, atmospheric, “painted”

---

## 11) Light direction: why we pass it, and how it affects both paths

* The core of diffuse is **Lambert**: `dot(N, L)`.
* We **normalize `L` on the CPU** to keep intensities meaningful and consistent.
* **Gouraud path:** computes Lambert at vertices with the same `L`; moving the sun changes vertex outputs, interpolated across the tri—**yes, it is affected by light direction**.
* **Per-fragment path:** uses the same `L` for diffuse and specular, and `V` for rim/spec; moving the sun repositions highlights and shadowed orientations **continuously**.

---

## 12) Practical implementation caveats (Bevy / encase / WGSL)

* **std140 alignment:** Arrays in uniform buffers must have **16-byte strides**. Our `TileUniform` packs to 16 bytes, and the struct/array is `#[repr(C, align(16))]`. This prevents the dreaded *“array stride must be a multiple of 16”* panic.
* **Counts:** 13×13 = 169 elements; still tiny vs UBO limits.
* **Binding order:** Keep sampler/texture arrays/uniforms in WGSL aligned with Rust `#[texture(..)]/#[sampler(..)]/#[uniform(..)]` indices.
* **CPU-side normalization:** `scene.light_direction` must be unit length.
* **Edge blend:** Implement as a function `edge_blend = smoothstep(0, margin_px, distance_to_edge)` used to mix normal bases near borders.
* **LOD idea:** For distant chunks, drop to **Gouraud diffuse**; for near/hero chunks, enable **per-fragment** + KR features.

---

## 13) FAQ quick hits

**Q: Is 13×13 *visibly* better than 12×12?**
**A:** With only diffuse, not much. With KR stack (rim/spec/fill + tonemap), yes—border consistency improves, bent-normal probes don’t downshift kernels near edges, highlights/fill gradients stay smooth across chunk seams.

**Q: Are bicubic and bent normals the same thing?**
**A:** No. **Bicubic** reconstructs a smooth surface from samples (a *geometric* model). **Bent** normals bias direction toward open sky (an *ambient visibility* model). We often compute bent normals **from the bicubic field** or blend between them.

**Q: Is “Phong shading” just any per-fragment lighting?**
**A:** Historically, **Phong shading** means per-fragment normal interpolation paired with the **Phong reflection** (Lambert diffuse + Phong spec). We’re technically doing **per-fragment** with **Blinn-Phong** specular for efficiency and smoother lobes.

**Q: Why tonemap if textures are 8-bit?**
**A:** Lighting pushes results > 1.0 (HDR), especially with rim/spec/fill and diffuse shaping. Tonemapping compresses HDR → SDR with a soft shoulder so highlights don’t clip.

**Q: Which features are “incompatible”?**
**A:** The big pitfalls: **Gouraud + rim/spec** (view-dependent terms die under interpolation), **geometric normals + rim/spec** (flicker at tile steps), **bent normals + geometric** (concept mismatch). See the compatibility table.

---

## Recommended defaults

* **Classic:** geometric + Gouraud Lambert; grading/tonemap off.
* **Enhanced Classic:** bicubic + per-fragment Lambert; weak fill; Reinhard tonemap (mild).
* **KR:** bicubic + bent for ambient; per-fragment Lambert + rim (subtle) + Blinn-Phong spec (low, wide); stronger fill; cloud/fog on; warm/cool grading; Reinhard tonemap.
