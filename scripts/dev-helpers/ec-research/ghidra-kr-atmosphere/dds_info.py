"""Print DDS header facts (size, mip count, FourCC) and PNG luminance statistics. Read only.

Usage: python3 -I dds_info.py FILE.dds|FILE.png ...
"""
import struct
import sys

DDS_MAGIC = b"DDS "
HEADER_HEIGHT_OFFSET = 12
HEADER_WIDTH_OFFSET = 16
HEADER_MIPS_OFFSET = 28
HEADER_FLAGS_OFFSET = 8
HEADER_FOURCC_OFFSET = 84
HEADER_CAPS_OFFSET = 108
DDSD_MIPMAPCOUNT = 0x20000
DDS_HEADER_READ = 128
MAX_CHANNEL = 255.0
CHANNELS = 3
PERCENTILE_DIVISOR = 10


def dds_info(path):
    with open(path, "rb") as handle:
        data = handle.read(DDS_HEADER_READ)
    if data[:4] != DDS_MAGIC:
        return None
    flags = struct.unpack_from("<I", data, HEADER_FLAGS_OFFSET)[0]
    height = struct.unpack_from("<I", data, HEADER_HEIGHT_OFFSET)[0]
    width = struct.unpack_from("<I", data, HEADER_WIDTH_OFFSET)[0]
    mips = struct.unpack_from("<I", data, HEADER_MIPS_OFFSET)[0]
    fourcc = data[HEADER_FOURCC_OFFSET:HEADER_FOURCC_OFFSET + 4]
    caps = struct.unpack_from("<I", data, HEADER_CAPS_OFFSET)[0]
    has_mips = bool(flags & DDSD_MIPMAPCOUNT)
    return width, height, mips if has_mips else 1, fourcc, hex(caps)


def main(paths):
    for path in paths:
        if path.endswith(".png"):
            from PIL import Image
            image = Image.open(path).convert("RGB")
            values = sorted(sum(pixel) / (CHANNELS * MAX_CHANNEL) for pixel in image.get_flattened_data())
            count = len(values)
            mean = sum(values) / count
            variance = sum((value - mean) ** 2 for value in values) / count
            print(path, "mean %.3f std %.3f min %.3f p10 %.3f p50 %.3f p90 %.3f max %.3f" % (
                mean, variance ** 0.5, values[0], values[count // PERCENTILE_DIVISOR], values[count // 2],
                values[count * (PERCENTILE_DIVISOR - 1) // PERCENTILE_DIVISOR], values[-1]))
        else:
            print(path, dds_info(path))


if __name__ == "__main__":
    main(sys.argv[1:])
