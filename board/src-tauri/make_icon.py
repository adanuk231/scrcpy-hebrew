"""The board's icon: two phones shoulder to shoulder, the way the magnet
leaves them. Drawn once per size rather than downscaled from one, so 16px
stays two clean blocks of colour instead of grey mush."""
from PIL import Image, ImageDraw

BLUE = (122, 178, 255, 255)
AMBER = (255, 199, 89, 255)
DARK = (14, 18, 27, 255)
SIZES = [16, 24, 32, 48, 64, 128, 256]


def phone_pair(s):
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    pad_x = max(1, round(s * 0.07))
    pad_y = max(1, round(s * 0.10))
    seam = max(1, round(s * 0.055))
    radius = max(1, round(s * 0.13))
    width = (s - pad_x * 2 - seam) // 2

    left = (pad_x, pad_y, pad_x + width, s - pad_y)
    right = (pad_x + width + seam, pad_y, pad_x + width * 2 + seam, s - pad_y)
    d.rounded_rectangle(left, radius=radius, fill=BLUE)
    d.rounded_rectangle(right, radius=radius, fill=AMBER)

    if s >= 64:
        inset = round(s * 0.035)
        for box in (left, right):
            d.rounded_rectangle(
                (box[0] + inset, box[1] + inset * 2, box[2] - inset, box[3] - inset * 2),
                radius=max(1, radius - inset), outline=DARK,
                width=max(1, round(s * 0.018)))
    return img


if __name__ == "__main__":
    frames = [phone_pair(n) for n in SIZES]
    frames[-1].save("icons/icon.ico", format="ICO",
                    sizes=[(n, n) for n in SIZES], append_images=frames[:-1])
    frames[-1].save("icons/icon.png")
    print("icons written:", SIZES)
