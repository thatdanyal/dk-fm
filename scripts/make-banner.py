"""Makes docs/banner.png (1280x640: the README header and GitHub's social preview) from
docs/logo.png and DK.FM's own fonts. Run from the repo root: python scripts/make-banner.py"""
from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H = 1280, 640
bg = Image.new("RGB", (W, H), (6, 8, 12))
glow = Image.new("RGB", (W, H), (0, 0, 0))
ImageDraw.Draw(glow).ellipse((60, 90, 560, 590), fill=(0, 120, 140))
bg = Image.blend(bg, glow.filter(ImageFilter.GaussianBlur(120)), 0.45)
d = ImageDraw.Draw(bg)
for y in range(0, H, 4):  # CRT scanlines
    d.line((0, y, W, y), fill=(0, 0, 0))
logo = Image.open("docs/logo.png").resize((400, 400), Image.LANCZOS)
bg.paste(logo, (90, 120), logo)
px = ImageFont.truetype("assets/PressStart2P-Regular.ttf", 84)
vt = ImageFont.truetype("assets/VT323-Regular.ttf", 46)
vt2 = ImageFont.truetype("assets/VT323-Regular.ttf", 36)
# the title in the logo's teal-to-blue
mask = Image.new("L", (W, H), 0)
ImageDraw.Draw(mask).text((560, 200), "DK.FM", font=px, fill=255)
grad = Image.new("RGB", (W, H))
gd = ImageDraw.Draw(grad)
for y in range(H):
    k = min(max((y - 200) / 90, 0), 1)
    gd.line((0, y, W, y), fill=(int(10 + 20 * k), int(230 - 60 * k), int(215 + 30 * k)))
bg.paste(grad, (0, 0), mask)
d = ImageDraw.Draw(bg)
d.text((562, 330), "Retro desktop music player", font=vt, fill=(220, 235, 245))
d.text((562, 385), "Imports Spotify, YouTube & SoundCloud", font=vt2, fill=(130, 165, 200))
d.text((562, 425), "Lyrics · THEATER · Sing-along · Album Cover themes", font=vt2, fill=(130, 165, 200))
d.text((562, 500), "Windows · macOS · Linux   —   tiny, native, free", font=vt2, fill=(90, 200, 210))
bg.save("docs/banner.png", optimize=True)
