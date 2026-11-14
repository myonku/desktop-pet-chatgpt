"""Pixel operations for straight RGBA frames."""

import cv2
import numpy as np
from PIL import Image


def optimize_frame(frame, *, keep_alpha=False, protected=(), denoise=False):
    result, removed = (frame.copy(), 0) if keep_alpha else clean_alpha(frame, protected)
    if denoise:
        result = denoise_rgb(result)
    return result, removed


def clear_hidden_rgb(frame):
    """Canonicalize fully transparent pixels before lossless compression."""
    rgba = np.array(frame)
    rgba[rgba[:, :, 3] == 0, :3] = 0
    return Image.fromarray(rgba)


def suppress_alpha_noise(frame, neighbors, protected=()):
    """Remove weak matte and small, detached components that flash for one frame."""
    rgba = np.array(frame)
    alpha = rgba[:, :, 3]
    unit = min(alpha.shape) / 320
    count, labels, stats, _ = cv2.connectedComponentsWithStats(
        (alpha >= 24).astype(np.uint8), connectivity=8
    )
    if count <= 1:
        return frame.copy(), 0
    areas = stats[:, cv2.CC_STAT_AREA]
    body = labels == 1 + areas[1:].argmax()
    if np.count_nonzero(body & (alpha >= 192)) < 64 * unit**2:
        return frame.copy(), 0
    guard = max(1, round(3 * unit))
    halo = cv2.dilate(body.astype(np.uint8), np.ones((2 * guard + 1,) * 2, np.uint8)) != 0
    support = np.zeros_like(alpha, dtype=np.uint8)
    for neighbor in neighbors:
        support |= np.asarray(neighbor.getchannel("A")) >= 24
    support = cv2.dilate(support, np.ones((2 * guard + 1,) * 2, np.uint8)) != 0
    overlap = np.bincount(labels[support], minlength=count)
    transient = (areas <= max(1, round(16 * unit**2))) & (overlap == 0)
    transient[0] = False
    remove = ~halo & ~support & ((alpha > 0) & (alpha < 24) | transient[labels])
    for x, y, width, height in protected:
        remove[y:y + height, x:x + width] = False
    alpha[remove] = 0
    return Image.fromarray(rgba), int(np.count_nonzero(remove))


def suppress_contour_matte(frame, protected=()):
    """Clear dark and yellow/olive matte outside the sprite's supported contour."""
    rgba = np.array(frame)
    alpha = rgba[:, :, 3]
    red, green, blue = np.moveaxis(rgba[:, :, :3].astype(np.int16), 2, 0)
    high = np.maximum(np.maximum(red, green), blue)
    low = np.minimum(np.minimum(red, green), blue)
    olive = (high <= 212) & (high - low <= 105) & (np.minimum(red, green) - blue >= 5)
    dark = (high <= 120) & ((high <= 55) | (blue - green <= 12))
    matte = olive | dark
    solid = (alpha >= 128) & ~matte
    count, labels, stats, _ = cv2.connectedComponentsWithStats(solid.astype(np.uint8), connectivity=8)
    unit = min(alpha.shape) / 320
    if count <= 1 or stats[1:, cv2.CC_STAT_AREA].max() < 64 * unit**2:
        return frame.copy(), 0

    # Keep antialiasing next to genuine color and protect dark enclosed detail.
    # The connected exterior can contain opaque matte pixels, so its flood
    # boundary is the trusted color mask rather than the source alpha mask.
    radius = max(1, round(unit))
    support = cv2.dilate(solid.astype(np.uint8), cv2.getStructuringElement(
        cv2.MORPH_ELLIPSE, (2 * radius + 1, 2 * radius + 1)
    ))
    exterior = np.pad((support == 0).astype(np.uint8), 1, constant_values=1)
    cv2.floodFill(exterior, None, (0, 0), 2, flags=8)
    remove = (alpha > 0) & matte & (exterior[1:-1, 1:-1] == 2)
    for x, y, width, height in protected:
        remove[y:y + height, x:x + width] = False
    alpha[remove] = 0
    return Image.fromarray(rgba), int(np.count_nonzero(remove))


def clean_alpha(frame, protected=()):
    """Clear detached, translucent matte pixels while retaining the visible body."""
    rgba = np.array(frame)
    alpha = rgba[:, :, 3]
    opaque = alpha >= 240
    if not opaque.any():
        return frame.copy(), 0

    # Treat the largest sufficiently visible component as the body, including
    # its translucent contour. Measure distance to every opaque component.
    count, labels, stats, _ = cv2.connectedComponentsWithStats(
        (alpha >= 16).astype(np.uint8), connectivity=8
    )
    if count <= 1:
        return frame.copy(), 0
    body = labels == 1 + stats[1:, cv2.CC_STAT_AREA].argmax()
    near_body = cv2.dilate(body.astype(np.uint8), np.ones((5, 5), np.uint8)) != 0
    distance = cv2.distanceTransform((~opaque).astype(np.uint8), cv2.DIST_L2, cv2.DIST_MASK_PRECISE)

    red, green, blue = np.moveaxis(rgba[:, :, :3].astype(np.int16), 2, 0)
    high = rgba[:, :, :3].max(axis=2)
    low = rgba[:, :, :3].min(axis=2)
    gray = (high <= 165) & (high - low <= 12)
    olive = (high <= 190) & (np.minimum(red, green) - blue >= 7) & (high - low <= 45)
    remove = (alpha > 0) & (alpha < 240) & (distance > 2) & ~near_body & (gray | olive)
    for x, y, width, height in protected:
        remove[y:y + height, x:x + width] = False
    alpha[remove] = 0
    return Image.fromarray(rgba), int(np.count_nonzero(remove))


def denoise_rgb(frame):
    """Apply a mild bilateral filter only inside fully opaque regions."""
    rgba = np.array(frame)
    rgb = rgba[:, :, :3].copy()
    interior = cv2.erode(
        (rgba[:, :, 3] == 255).astype(np.uint8), np.ones((5, 5), np.uint8)
    ).astype(bool)
    filtered = cv2.bilateralFilter(rgb, d=5, sigmaColor=12, sigmaSpace=2)
    rgba[:, :, :3][interior] = filtered[interior]
    return Image.fromarray(rgba)


def rgb_for_upscale(frame):
    """Extend the nearest visible RGB into transparent pixels before inference."""
    rgba = np.array(frame)
    rgb = rgba[:, :, :3].copy()
    visible = rgba[:, :, 3] != 0
    if not visible.any():
        return Image.new("RGB", frame.size)
    _, labels = cv2.distanceTransformWithLabels(
        (~visible).astype(np.uint8), cv2.DIST_L2, 5, labelType=cv2.DIST_LABEL_PIXEL
    )
    colors = np.empty((int(labels.max()) + 1, 3), dtype=np.uint8)
    colors[labels[visible]] = rgb[visible]
    return Image.fromarray(colors[labels])
