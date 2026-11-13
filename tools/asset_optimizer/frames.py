"""Pixel operations for straight RGBA frames."""

import cv2
import numpy as np
from PIL import Image


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
