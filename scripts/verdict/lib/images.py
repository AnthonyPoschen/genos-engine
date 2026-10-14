"""Picture differences for verdicts.

FLIP here follows the same departures as `genos-lighting-ref`: a Gaussian in
YCxCy, a Sobel feature, and `color ** (1 - feature)`. It is not NVIDIA's tool.
Mean absolute error is the channel mean of |a - b| on 0..1 RGB.
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

from lib.contract import Broken

PPD = 67.0
SIGMA_Y = 0.04 * PPD
SIGMA_C = 0.15 * PPD


class Image:
    def __init__(self, width: int, height: int, rgb: list[float]) -> None:
        need = width * height * 3
        if width < 1 or height < 1 or len(rgb) != need:
            raise Broken(f"picture {width}x{height} needs {need} channels, got {len(rgb)}")
        self.width = width
        self.height = height
        self.rgb = rgb

    def pixels(self) -> int:
        return self.width * self.height

    def at(self, x: int, y: int) -> tuple[float, float, float]:
        i = (y * self.width + x) * 3
        return self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]


def luma(rgb: tuple[float, float, float]) -> float:
    return 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]


def mean_abs(reference: Image, candidate: Image) -> float:
    _same_size(reference, candidate)
    total = 0.0
    for a, b in zip(reference.rgb, candidate.rgb, strict=True):
        total += abs(a - b)
    return total / len(reference.rgb)


def blur_relative(reference: Image, candidate: Image, radius: int = 2) -> float:
    _same_size(reference, candidate)
    ref = _box(_luma_plane(reference), reference.width, reference.height, radius)
    cand = _box(_luma_plane(candidate), candidate.width, candidate.height, radius)
    mean_ref = sum(ref) / len(ref) if ref else 0.0
    floor = max(0.25 * mean_ref, 0.02)
    total = 0.0
    for left, right in zip(ref, cand, strict=True):
        total += abs(right - left) / max(left, floor)
    return total / len(ref)


def flip_mean_p99(reference: Image, candidate: Image) -> tuple[float, float]:
    values = _flip_map(reference, candidate)
    if not values:
        return 0.0, 0.0
    mean = sum(values) / len(values)
    ordered = sorted(values)
    rank = math.ceil(0.99 * len(ordered))
    index = min(len(ordered) - 1, max(0, rank - 1))
    return mean, ordered[index]


def diff_image(reference: Image, candidate: Image) -> Image:
    _same_size(reference, candidate)
    out: list[float] = []
    peak = 0.0
    for a, b in zip(reference.rgb, candidate.rgb, strict=True):
        value = abs(a - b)
        out.append(value)
        peak = max(peak, value)
    if peak > 0:
        out = [value / peak for value in out]
    return Image(reference.width, reference.height, out)


def write_png(path: Path, image: Image) -> None:
    raw = bytearray()
    stride = image.width * 3
    for y in range(image.height):
        raw.append(0)
        for value in image.rgb[y * stride : (y + 1) * stride]:
            raw.append(max(0, min(255, int(round(value * 255.0)))))
    compressed = zlib.compress(bytes(raw))
    ihdr = struct.pack(">IIBBBBB", image.width, image.height, 8, 2, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", ihdr) + _chunk(b"IDAT", compressed) + _chunk(b"IEND", b"")
    path.write_bytes(png)


def load_image(path: Path) -> Image:
    data = path.read_bytes()
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return _load_png(data)
    if data.startswith(b"P6"):
        return _load_ppm(data)
    raise Broken(f"unsupported image {path}")


def solid(width: int, height: int, rgb: tuple[float, float, float]) -> Image:
    channels: list[float] = []
    for _ in range(width * height):
        channels.extend(rgb)
    return Image(width, height, channels)


def _same_size(reference: Image, candidate: Image) -> None:
    if reference.width != candidate.width or reference.height != candidate.height:
        raise Broken("pictures differ in size")


def _luma_plane(image: Image) -> list[float]:
    return [luma(image.at(x, y)) for y in range(image.height) for x in range(image.width)]


def _box(src: list[float], width: int, height: int, radius: int) -> list[float]:
    if radius < 1:
        return list(src)
    out = [0.0] * len(src)
    for y in range(height):
        for x in range(width):
            total = 0.0
            count = 0
            for yy in range(max(0, y - radius), min(height, y + radius + 1)):
                for xx in range(max(0, x - radius), min(width, x + radius + 1)):
                    total += src[yy * width + xx]
                    count += 1
            out[y * width + x] = total / count
    return out


def _flip_map(reference: Image, candidate: Image) -> list[float]:
    _same_size(reference, candidate)
    y0, cx0, cy0 = _planes(reference)
    y1, cx1, cy1 = _planes(candidate)
    width, height = reference.width, reference.height
    y0b = _gauss(y0, width, height, SIGMA_Y)
    y1b = _gauss(y1, width, height, SIGMA_Y)
    cx0b = _gauss(cx0, width, height, SIGMA_C)
    cx1b = _gauss(cx1, width, height, SIGMA_C)
    cy0b = _gauss(cy0, width, height, SIGMA_C)
    cy1b = _gauss(cy1, width, height, SIGMA_C)
    e0 = _sobel(y0, width, height)
    e1 = _sobel(y1, width, height)
    out: list[float] = []
    for i in range(reference.pixels()):
        dy = (y1b[i] - y0b[i]) / 0.20
        dx = (cx1b[i] - cx0b[i]) / 0.10
        dz = (cy1b[i] - cy0b[i]) / 0.10
        color = 1.0 - math.exp(-math.sqrt(dy * dy + dx * dx + dz * dz))
        feature = abs(e1[i] - e0[i]) / (max(e0[i], e1[i]) + 0.05)
        feature = min(1.0, max(0.0, feature))
        out.append(min(1.0, max(0.0, color ** (1.0 - feature))))
    return out


def _planes(image: Image) -> tuple[list[float], list[float], list[float]]:
    y: list[float] = []
    cx: list[float] = []
    cy: list[float] = []
    for row in range(image.height):
        for col in range(image.width):
            red, green, blue = image.at(col, row)
            y.append(luma((red, green, blue)))
            cx.append(0.5 * (red - green))
            cy.append(0.5 * (0.5 * (red + green) - blue))
    return y, cx, cy


def _gauss(src: list[float], width: int, height: int, sigma: float) -> list[float]:
    if sigma < 0.25 or width == 0 or height == 0:
        return list(src)
    radius = int(math.ceil(3.0 * sigma))
    kernel: list[float] = []
    total = 0.0
    for i in range(-radius, radius + 1):
        weight = math.exp(-0.5 * (i / sigma) ** 2)
        kernel.append(weight)
        total += weight
    kernel = [weight / total for weight in kernel]
    tmp = [0.0] * len(src)
    for y in range(height):
        for x in range(width):
            acc = 0.0
            for k, weight in enumerate(kernel):
                xx = min(width - 1, max(0, x + k - radius))
                acc += src[y * width + xx] * weight
            tmp[y * width + x] = acc
    out = [0.0] * len(src)
    for y in range(height):
        for x in range(width):
            acc = 0.0
            for k, weight in enumerate(kernel):
                yy = min(height - 1, max(0, y + k - radius))
                acc += tmp[yy * width + x] * weight
            out[y * width + x] = acc
    return out


def _sobel(src: list[float], width: int, height: int) -> list[float]:
    out = [0.0] * len(src)
    if width < 3 or height < 3:
        return out

    def at(x: int, y: int) -> float:
        xx = min(width - 1, max(0, x))
        yy = min(height - 1, max(0, y))
        return src[yy * width + xx]

    for y in range(height):
        for x in range(width):
            gx = (
                -at(x - 1, y - 1)
                + at(x + 1, y - 1)
                - 2.0 * at(x - 1, y)
                + 2.0 * at(x + 1, y)
                - at(x - 1, y + 1)
                + at(x + 1, y + 1)
            )
            gy = (
                -at(x - 1, y - 1)
                - 2.0 * at(x, y - 1)
                - at(x + 1, y - 1)
                + at(x - 1, y + 1)
                + 2.0 * at(x, y + 1)
                + at(x + 1, y + 1)
            )
            out[y * width + x] = math.sqrt(gx * gx + gy * gy)
    return out


def _chunk(tag: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)


def _load_png(data: bytes) -> Image:
    pos = 8
    width = height = 0
    color = 0
    idat = bytearray()
    while pos + 8 <= len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        tag = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if tag == b"IHDR":
            width, height, depth, color, _comp, _filter, interlace = struct.unpack(">IIBBBBB", chunk)
            if depth != 8 or color not in (2, 6) or interlace != 0:
                raise Broken("png must be 8-bit rgb or rgba, not interlaced")
        elif tag == b"IDAT":
            idat.extend(chunk)
        elif tag == b"IEND":
            break
    if width < 1 or height < 1 or not idat:
        raise Broken("png is incomplete")
    raw = zlib.decompress(bytes(idat))
    channels = 3 if color == 2 else 4
    stride = width * channels
    rgb: list[float] = []
    previous = [0] * stride
    cursor = 0
    for _y in range(height):
        if cursor >= len(raw):
            raise Broken("png rows are short")
        filt = raw[cursor]
        row = list(raw[cursor + 1 : cursor + 1 + stride])
        cursor += 1 + stride
        if len(row) != stride:
            raise Broken("png row is short")
        current = _unfilter(filt, row, previous, channels)
        previous = current
        for x in range(width):
            i = x * channels
            rgb.extend(channel / 255.0 for channel in current[i : i + 3])
    return Image(width, height, rgb)


def _unfilter(filt: int, row: list[int], previous: list[int], bpp: int) -> list[int]:
    out = [0] * len(row)
    for i, value in enumerate(row):
        left = out[i - bpp] if i >= bpp else 0
        up = previous[i]
        ul = previous[i - bpp] if i >= bpp else 0
        if filt == 0:
            current = value
        elif filt == 1:
            current = value + left
        elif filt == 2:
            current = value + up
        elif filt == 3:
            current = value + (left + up) // 2
        elif filt == 4:
            current = value + _paeth(left, up, ul)
        else:
            raise Broken(f"png filter {filt}")
        out[i] = current & 0xFF
    return out


def _paeth(left: int, up: int, ul: int) -> int:
    estimate = left + up - ul
    dl = abs(estimate - left)
    du = abs(estimate - up)
    dul = abs(estimate - ul)
    if dl <= du and dl <= dul:
        return left
    if du <= dul:
        return up
    return ul


def _load_ppm(data: bytes) -> Image:
    lines = data.split(b"\n", 2)
    if len(lines) < 3 or lines[0] != b"P6":
        raise Broken("ppm must be P6")
    size = lines[1].split()
    if len(size) < 3:
        raise Broken("ppm header is short")
    width, height, depth = int(size[0]), int(size[1]), int(size[2])
    if depth != 255:
        raise Broken("ppm depth must be 255")
    raw = lines[2]
    need = width * height * 3
    if len(raw) < need:
        raise Broken("ppm pixels are short")
    return Image(width, height, [byte / 255.0 for byte in raw[:need]])
