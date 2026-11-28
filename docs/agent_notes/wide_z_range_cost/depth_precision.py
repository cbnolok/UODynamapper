"""f32 emulation of art/depth.wesl's draw-order depth for wider z ranges.

Evaluates priority_logical_height -> logical_view_z_from -> projected_view_z_depth in float32 (each operation
rounded, and a variant with fused multiply-adds), over the logical points a 1900x1000 view can show at each zoom,
and reports how far apart adjacent keys land in units in the last place (ulp) of the depth, and how many adjacent
pairs collide (equal depth) or invert.

Run: python3 -I -B depth_precision.py
"""
import numpy as np

F = np.float32
SQRT3 = np.sqrt(3.0)
WINDOW_HEIGHT = 1000.0
ORTHO_SIZE_FACTOR = 44.0 / np.sqrt(2.0)
PERSPECTIVE_FOV = 0.25
PERSPECTIVE_NEAR = F(0.1)
ORTHO_OFFSET = 5.0
ORTHO_NEAR, ORTHO_FAR = -1000.0, 1000.0
TIE_LEVELS = 9
UO_Z_TO_WORLD = 4.0 / 44.0
# Screen pixels per diagonal row and per z at zoom 1.
PX_PER_ROW = 22.0
PX_PER_Z = 4.0
HALF_SCREEN_ROWS = WINDOW_HEIGHT / 2.0 / PX_PER_ROW
CAMERA_FRACTION = 0.37  # a walking player's sub-cell position
# Samples of the priority z range and of the visible rows.
PZ_SAMPLES = 97
ROW_SAMPLES = 801


def orbit_distance(zoom):
    return WINDOW_HEIGHT * zoom / (2.0 * np.tan(PERSPECTIVE_FOV / 2.0) * ORTHO_SIZE_FACTOR)


def fl(x):
    return np.asarray(x, dtype=np.float32)


def view_z(cells_x, cells_y, lh, cam, axis, fma):
    rx = fl(cells_x) - cam[0]
    ry = fl(lh) - cam[1]
    rz = fl(cells_y) - cam[2]
    if fma:
        # fma(az, rz, fma(ay, ry, ax * rx)): one rounding per fused step.
        t = (axis[0] * rx).astype(np.float32)
        t = (np.float64(axis[1]) * ry.astype(np.float64) + t.astype(np.float64)).astype(np.float32)
        return (np.float64(axis[2]) * rz.astype(np.float64) + t.astype(np.float64)).astype(np.float32)
    return ((axis[0] * rx + axis[1] * ry).astype(np.float32) + axis[2] * rz).astype(np.float32)


def depth_of(vz, projection, fma):
    p22, p32, p23, p33 = projection
    if fma:
        w = (np.float64(p23) * vz.astype(np.float64) + np.float64(p33)).astype(np.float32)
        n = (np.float64(p22) * vz.astype(np.float64) + np.float64(p32)).astype(np.float32)
    else:
        w = (p23 * vz + p33).astype(np.float32)
        n = (p22 * vz + p32).astype(np.float32)
    return (n / w).astype(np.float32)


def logical_height(pz, level, steps):
    return ((fl(pz) + fl(level) / F(TIE_LEVELS)) / F(steps)).astype(np.float32)


def analyse(name, steps, z_half, zoom, perspective, focus, player_z, fit=None, fma=False):
    """Adjacent-key separation over the visible logical points.

    steps: PRIORITY_Z_STEPS_PER_ROW; z_half: the z range's half width (128 for i8); fit: None (today's +-1000 /
    infinite perspective), 'ortho-fit' (the near/far fitted to the visible logical view z range) or 'focus-fit'
    (the view z taken from the focus and mapped linearly over the visible range, for the perspective camera)."""
    z_range = 2 * z_half
    rows = int(np.ceil(HALF_SCREEN_ROWS * zoom + PX_PER_Z * z_range / PX_PER_ROW)) + 2
    distance = orbit_distance(zoom) if perspective else ORTHO_OFFSET * SQRT3
    player = np.array([focus[0] + CAMERA_FRACTION, player_z * UO_Z_TO_WORLD, focus[1] + CAMERA_FRACTION])
    cam64 = player + np.ones(3) / SQRT3 * distance
    if fit == 'focus-fit':
        # View z taken from the focus instead of the camera (the camera's distance is a constant of the mapping),
        # mapped linearly over the visible range: the same fitted mapping in both cameras.
        cam64 = player
    cam = fl(cam64)
    axis = fl(np.ones(3) / SQRT3)
    # Priority z: the land's lowest - 2 to the highest static's + 2, every 7th value and the ends.
    pz = np.unique(np.concatenate([np.round(np.linspace(-z_half - 2, z_half + 1, PZ_SAMPLES)), [-2.0, 0.0, 1.0]]))
    r = np.unique(np.round(np.linspace(-rows, rows, min(2 * rows + 1, ROW_SAMPLES))))
    cx = np.floor(focus[0]) + np.floor(r / 2.0)
    cy = np.floor(focus[1]) + r - np.floor(r / 2.0)
    # Logical view z of the visible range, for the fitted projection.
    if perspective and fit is None:
        projection = (F(0.0), PERSPECTIVE_NEAR, F(-1.0), F(0.0))
    else:
        if fit in ('ortho-fit', 'focus-fit'):
            lo_l = -z_half / steps
            hi_l = (z_half + 1) / steps
            corners = []
            for rr in (r[0], r[-1]):
                for lh in (lo_l, hi_l):
                    x = np.floor(focus[0]) + np.floor(rr / 2.0)
                    y = np.floor(focus[1]) + rr - np.floor(rr / 2.0)
                    corners.append(((x - cam64[0]) + (lh - cam64[1]) + (y - cam64[2])) / SQRT3)
            vmin, vmax = min(corners) - 1.0, max(corners) + 1.0
            # Reverse Z: vmax (nearest) -> 1, vmin -> 0.
            k = 1.0 / (vmax - vmin)
            projection = (F(k), F(-vmin * k), F(0.0), F(1.0))
        else:
            rcp = 1.0 / (ORTHO_FAR - ORTHO_NEAR)
            projection = (F(rcp), F(rcp * ORTHO_FAR), F(0.0), F(1.0))
    X, P, T = np.meshgrid(cx, pz, np.arange(TIE_LEVELS), indexing='ij')
    Y, _, _ = np.meshgrid(cy, pz, np.arange(TIE_LEVELS), indexing='ij')
    d = depth_of(view_z(X, Y, logical_height(P, T, steps), cam, axis, fma), projection, fma)
    # Adjacent tie levels within one priority z, and the last level of pz against level 0 of the next sampled pz
    # (sampled pz are 7 apart, so that pair is far apart: only the level pairs are the margin).
    a = d[:, :, :-1].ravel()
    b = d[:, :, 1:].ravel()
    ulp = np.spacing(np.maximum(a, b)).astype(np.float64)
    sep = (b.astype(np.float64) - a.astype(np.float64)) / ulp  # reverse Z: the higher level is nearer, larger depth
    collide = float(np.mean(sep == 0))
    invert = float(np.mean(sep < 0))
    # Exact separation of one tie level, in ulps of the depth where the depth is coarsest.
    worst_ulp = float(np.max(np.spacing(d)))
    level_view = 1.0 / (TIE_LEVELS * steps) / SQRT3
    if perspective and fit is None:
        vz_far = -(distance + rows / SQRT3)
        exact = PERSPECTIVE_NEAR * level_view / vz_far ** 2
        exact_ulp = exact / float(np.spacing(F(PERSPECTIVE_NEAR / -vz_far)))
    else:
        exact_ulp = level_view * projection[0] / worst_ulp
    return dict(name=name, zoom=zoom, rows=rows, pairs=a.size, min_sep=float(sep.min()), collide=collide,
                invert=invert, level_ulps=exact_ulp)


def main():
    focuses = {'near origin (100,100)': (100.0, 100.0), 'Britain (1496,1628)': (1496.0, 1628.0),
               'far corner (7100,4050)': (7100.0, 4050.0)}
    options = [('i8, S 264 (today)', 264.0, 128), ('9-bit, S 264', 264.0, 256), ('9-bit, S 520', 520.0, 256),
               ('10-bit, S 264', 264.0, 512), ('10-bit, S 520', 520.0, 512), ('10-bit, S 1032', 1032.0, 512),
               ('i16, S 264', 264.0, 32768)]
    zooms = [1.0, 4.0, 10.0, 20.0]
    titles = {(False, None): 'orthographic, near/far +-1000 (today)',
              (False, 'ortho-fit'): 'orthographic, near/far fitted to the visible logical range',
              (True, None): 'perspective, infinite reverse Z, near 0.1 (today)',
              (True, 'focus-fit'): 'perspective, view z from the focus, mapped linearly over the visible logical range'}
    for (perspective, fit), cam in titles.items():
        print(f'\n## {cam}')
        print('| option | zoom | rows +- | 1 level in ulps (exact) | min adjacent separation, ulps | collisions | '
              'inversions |')
        print('|---|---:|---:|---:|---:|---:|---:|')
        for label, steps, z_half in options:
            for zoom in zooms:
                worst = None
                for focus_name, focus in focuses.items():
                    for player_z in (0.0, z_half - 1.0):
                        for fma in (False, True):
                            res = analyse(label, steps, z_half, zoom, perspective, focus, player_z, fit, fma)
                            if worst is None or res['min_sep'] < worst['min_sep'] or res['collide'] + res['invert'] > \
                                    worst['collide'] + worst['invert']:
                                worst = res
                print(f"| {label} | {zoom:g} | {worst['rows']} | {worst['level_ulps']:.2f} | {worst['min_sep']:.0f} | "
                      f"{100 * worst['collide']:.2f} % | {100 * worst['invert']:.2f} % |")


if __name__ == '__main__':
    main()
