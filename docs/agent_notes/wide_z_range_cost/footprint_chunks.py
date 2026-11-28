"""Follow-view chunk counts (footprint.rs compute_visible_chunks) at 1900x1000 for the pad today and the pad a
z range needs: a cell dz above or below the player's shifts dz/11 tiles along each axis on the ground plane, and
the tallest art (591 px) reaches 591/44 tiles. Averaged over sub-chunk camera positions."""
import math

W, H = 1900.0, 1000.0
TILE = 32
ORTHO_SIZE_FACTOR = 44.0 / math.sqrt(2.0)
ORTHO_WIDTH_SCALE_FACTOR = math.sqrt(3.0)
ART_TILES = 591.0 / 44.0


def half_extent(zoom):
    ow = W / ORTHO_SIZE_FACTOR
    oh = (H / ORTHO_WIDTH_SCALE_FACTOR) / ORTHO_SIZE_FACTOR
    return ow * zoom / 2 / math.sqrt(2) + oh * zoom / 2 * 3 / math.sqrt(6)


def scale_from_zoom(zoom):
    for limit, scale in ((50, 32), (35, 16), (20, 8), (10, 4), (5, 2)):
        if zoom >= limit:
            return scale
    return 1


def pad_today(scale):
    return TILE * (2 if scale <= 2 else scale + max(scale // 4, 1))


def chunks(zoom, pad):
    scale = scale_from_zoom(zoom)
    span = TILE * scale
    he = half_extent(zoom)
    total = 0
    samples = 16
    for i in range(samples):
        c = 4000.0 + span * i / samples
        n = math.floor((c + he + pad) / span) + 1 - math.floor((c - he - pad) / span)
        total += n * n
    return total / samples


def main():
    ranges = {'i8': 255, '9-bit': 511, '10-bit': 1023}
    print('| zoom | scale | half extent, tiles | pad today | chunks today | ' +
          ' | '.join(f'{k}: pad needed / chunks' for k in ranges) + ' |')
    for zoom in (1.0, 2.0, 4.0, 5.0, 9.0, 10.0, 20.0, 50.0):
        scale = scale_from_zoom(zoom)
        p = pad_today(scale)
        row = [f'{zoom:g}', str(scale), f'{half_extent(zoom):.0f}', str(p), f'{chunks(zoom, p):.0f}']
        for name, rng in ranges.items():
            need = rng / 11.0 + ART_TILES
            if need <= p:
                row.append(f'{need:.0f} / same')
            else:
                padded = math.ceil(need / (TILE * scale)) * TILE * scale
                row.append(f'{need:.0f} / {chunks(zoom, padded):.0f} at pad {padded}')
        print('| ' + ' | '.join(row) + ' |')


main()
