# /// script
# dependencies = ["fonttools==4.55.3", "brotli==1.1.0"]
# ///
"""The page's fonts, cut down and carried by the app (ui/fonts/, built into the engine by http.rs):

- Inter (the interface) and JetBrains Mono (numbers): Latin, digits, the punctuation and signs the page uses
- Noto Sans TC (Chinese): only the characters in the page's files (ui/*.html, ui/app.js); a device's name with others
  falls to the system's font

All three are variable (one file for every weight) and under the SIL Open Font License 1.1, whose text goes beside them.
Run again whenever the page's Chinese text changes.
Usage: uv run tools/make_fonts.py FONTS_DIR   (FONTS_DIR/inter/Inter[opsz,wght].ttf, .../jetbrainsmono/..., .../notosanstc/...,
each with its OFL.txt: google/fonts on GitHub)
"""
import glob
import os
import shutil
import sys

from fontTools import subset

SRC = sys.argv[1]
UI = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ui')
OUT = os.path.join(UI, 'fonts')
os.makedirs(OUT, exist_ok=True)

text = ''.join(open(f, encoding='utf-8').read() for f in glob.glob(os.path.join(UI, '*.html')) + [os.path.join(UI, 'app.js')])
latin = ''.join(chr(c) for c in range(0x20, 0x7F)) + ''.join(chr(c) for c in range(0xA0, 0x100)) + '–—‘’“”…·•−∞×÷±→←↑↓‹››≈≤≥°'
cjk = sorted({ch for ch in text if ord(ch) > 0x2000} | set('，。、：；！？（）「」『』《》〈〉…—・％＋－'))


def cut(src, out, chars):
    opts = subset.Options()
    opts.flavor = 'woff2'
    opts.layout_features = ['*']
    opts.name_IDs = ['*']
    opts.notdef_outline = True
    f = subset.load_font(src, opts)
    s = subset.Subsetter(opts)
    s.populate(text=''.join(chars))
    s.subset(f)
    subset.save_font(f, out, opts)
    print('%-28s %6d chars  %7d bytes' % (os.path.basename(out), len(chars), os.path.getsize(out)))


cut(glob.glob(SRC + '/inter/Inter*.ttf')[0], OUT + '/inter.woff2', latin)
cut(glob.glob(SRC + '/jetbrainsmono/JetBrainsMono*.ttf')[0], OUT + '/jetbrains-mono.woff2', latin)
cut(glob.glob(SRC + '/notosanstc/NotoSansTC*.ttf')[0], OUT + '/noto-sans-tc.woff2', cjk + list(latin))
for name, d in (('Inter', 'inter'), ('JetBrains-Mono', 'jetbrainsmono'), ('Noto-Sans-TC', 'notosanstc')):
    shutil.copy(os.path.join(SRC, d, 'OFL.txt'), os.path.join(OUT, 'OFL-%s.txt' % name))
