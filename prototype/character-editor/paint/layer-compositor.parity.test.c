/* Layer compositor parity test: does afterglow_layer_blend_over() produce
 * exactly the same pixels as MyPaint's layer stack?
 *
 * This file transcribes MyPaint's layer-stack source (fix15.hpp,
 * blending.hpp, compositing.hpp, and the tile_combine dispatch in
 * pixops.cpp) into plain C. It then exhaustively compares the reference
 * against our fixed-point compositor over a grid of source/backdrop
 * colours, alphas and opacities across all 22 modes.
 *
 * The reference here is DATA, not logic: it mirrors MyPaint's exact integer
 * arithmetic (truncating fix15 multiply/divide), so any mismatch with
 * layer-compositor.c is a porting error in our engine, not a rounding
 * judgement call. The only allowed tolerance is the Pigment (spectral WGM)
 * path, which uses float fastpow and may differ by a couple of LSBs.
 *
 * MyPaint is GPL-2.0-or-later (copyright Andrew Chadwick et al.). The
 * reference section below is a derivative transcription of its headers.
 * This test file is intentionally GPL; it is not linked into the engine.
 */

#include "layer-compositor.h"

#include <assert.h>
#include <stdint.h>
#include <stdio.h>

/* Same fastapprox fastpow as both libmypaint and MyPaint ship; using it on
 * the reference side keeps the Pigment comparison honest (identical pow). */
#include "vendor/libmypaint/fastapprox/fastpow.h"

/* MyPaint fix15.hpp ---------------------------------------------------- */
#define F15 32768u

static uint32_t f_mul(uint32_t a, uint32_t b) { return (a * b) >> 15; }
static uint32_t f_sumprods(uint32_t a1, uint32_t a2, uint32_t b1, uint32_t b2)
{ return ((a1 * a2) + (b1 * b2)) >> 15; }
static uint32_t f_div(uint32_t a, uint32_t b) { return (a << 15) / b; }
static uint32_t f_short_clamp(uint32_t n) { return n > F15 ? F15 : n; }

static uint32_t f_sqrt(uint32_t x)
{
    if (x == 0 || x == F15) return x;
    static const uint16_t approx16[16] = {
        16383, 23169, 28376, 32767, 36634, 40131, 43346, 46339,
        49151, 51809, 54338, 56754, 59072, 61302, 63453, 65535
    };
    uint32_t s = x << 1;
    const int fracbits = 16;
    uint32_t n = approx16[s >> 12];
    uint32_t n_old = 0;
    for (int i = 0; i < 15; ++i) {
        n_old = n;
        n += (s << fracbits) / n;
        n >>= 1;
        if (n == n_old
            || ((n > n_old) && (n - 1 == n_old))
            || ((n < n_old) && (n + 1 == n_old))) break;
    }
    return n >> 1;
}

/* MyPaint blending.hpp: spectral upsampling (WGM pigment) ---------------- */
#define WGM_EPSILON 0.001f
static const float T_MATRIX_SMALL[3][10] = {
    {0.026595621243689f,0.049779426257903f,0.022449850859496f,-0.218453689278271f,
     -0.256894883201278f,0.445881722194840f,0.772365886289756f,0.194498761382537f,
     0.014038157587820f,0.007687264480513f},
    {-0.032601672674412f,-0.061021043498478f,-0.052490001018404f,0.206659098273522f,
     0.572496335158169f,0.317837248815438f,-0.021216624031211f,-0.019387668756117f,
     -0.001521339050858f,-0.000835181622534f},
    {0.339475473216284f,0.635401374177222f,0.771520797089589f,0.113222640692379f,
     -0.055251113343776f,-0.048222578468680f,-0.012966666339586f,-0.001523814504223f,
     -0.000094718948810f,-0.000051604594741f}
};
static const float spectral_r_small[10] = {0.009281362787953f,0.009732627042016f,
    0.011254252737167f,0.015105578649573f,0.024797924177217f,0.083622585502406f,
    0.977865045723212f,1.000000000000000f,0.999961046144372f,0.999999992756822f};
static const float spectral_g_small[10] = {0.002854127435775f,0.003917589679914f,
    0.012132151699187f,0.748259205918013f,1.000000000000000f,0.865695937531795f,
    0.037477469241101f,0.022816789725717f,0.021747419446456f,0.021384940572308f};
static const float spectral_b_small[10] = {0.537052150373386f,0.546646402401469f,
    0.575501819073983f,0.258778829633924f,0.041709923751716f,0.012662638828324f,
    0.007485593127390f,0.006766900622462f,0.006699764779016f,0.006676219883241f};

static void rgb_to_spectral(float r, float g, float b, float *spectral_)
{
    const float offset = 1.0f - WGM_EPSILON;
    r = r * offset + WGM_EPSILON;
    g = g * offset + WGM_EPSILON;
    b = b * offset + WGM_EPSILON;
    float spec_r[10] = {0};
    float spec_g[10] = {0};
    float spec_b[10] = {0};
    for (int i = 0; i < 10; i++) spec_r[i] = spectral_r_small[i] * r;
    for (int i = 0; i < 10; i++) spec_g[i] = spectral_g_small[i] * g;
    for (int i = 0; i < 10; i++) spec_b[i] = spectral_b_small[i] * b;
    for (int i = 0; i < 10; i++) spectral_[i] += spec_r[i] + spec_g[i] + spec_b[i];
}

static void spectral_to_rgb(float *spectral, float *rgb_)
{
    const float offset = 1.0f - WGM_EPSILON;
    float tmp[3] = {0};
    for (int i = 0; i < 10; i++) {
        tmp[0] += T_MATRIX_SMALL[0][i] * spectral[i];
        tmp[1] += T_MATRIX_SMALL[1][i] * spectral[i];
        tmp[2] += T_MATRIX_SMALL[2][i] * spectral[i];
    }
    for (int i = 0; i < 3; i++) {
        float v = (tmp[i] - WGM_EPSILON) / offset;
        if (v < 0.0f) v = 0.0f;
        if (v > 1.0f) v = 1.0f;
        rgb_[i] = v;
    }
}

static float ref_fastpow(float a, float b)
{
    return fastpow(a, b);
}

/* MyPaint blending.hpp: blend functors (straight colour in/out) ---------- */
typedef void (*blendfn)(uint32_t, uint32_t, uint32_t,
                        uint32_t *, uint32_t *, uint32_t *);

static void b_normal(uint32_t sr, uint32_t sg, uint32_t sb,
                     uint32_t *dr, uint32_t *dg, uint32_t *db)
{ (void)sr; (void)sg; (void)sb; *dr = sr; *dg = sg; *db = sb; }
static void b_multiply(uint32_t sr, uint32_t sg, uint32_t sb,
                       uint32_t *dr, uint32_t *dg, uint32_t *db)
{ *dr = f_mul(sr, *dr); *dg = f_mul(sg, *dg); *db = f_mul(sb, *db); }
static void b_screen(uint32_t sr, uint32_t sg, uint32_t sb,
                     uint32_t *dr, uint32_t *dg, uint32_t *db)
{ *dr = *dr + sr - f_mul(*dr, sr); *dg = *dg + sg - f_mul(*dg, sg); *db = *db + sb - f_mul(*db, sb); }
static void b_overlay(uint32_t sr, uint32_t sg, uint32_t sb,
                      uint32_t *dr, uint32_t *dg, uint32_t *db)
{
    uint32_t t = *dr << 1;
    *dr = (t <= F15) ? f_mul(sr, t) : sr + (t - F15) - f_mul(sr, t - F15);
    t = *dg << 1;
    *dg = (t <= F15) ? f_mul(sg, t) : sg + (t - F15) - f_mul(sg, t - F15);
    t = *db << 1;
    *db = (t <= F15) ? f_mul(sb, t) : sb + (t - F15) - f_mul(sb, t - F15);
}
static void b_darken(uint32_t sr, uint32_t sg, uint32_t sb,
                     uint32_t *dr, uint32_t *dg, uint32_t *db)
{ if (sr < *dr) *dr = sr; if (sg < *dg) *dg = sg; if (sb < *db) *db = sb; }
static void b_lighten(uint32_t sr, uint32_t sg, uint32_t sb,
                      uint32_t *dr, uint32_t *dg, uint32_t *db)
{ if (sr > *dr) *dr = sr; if (sg > *dg) *dg = sg; if (sb > *db) *db = sb; }
static void b_hardlight(uint32_t sr, uint32_t sg, uint32_t sb,
                        uint32_t *dr, uint32_t *dg, uint32_t *db)
{
    uint32_t t = sr << 1;
    *dr = (t <= F15) ? f_mul(*dr, t) : *dr + (t - F15) - f_mul(*dr, t - F15);
    t = sg << 1;
    *dg = (t <= F15) ? f_mul(*dg, t) : *dg + (t - F15) - f_mul(*dg, t - F15);
    t = sb << 1;
    *db = (t <= F15) ? f_mul(*db, t) : *db + (t - F15) - f_mul(*db, t - F15);
}
static void softlight_channel(uint32_t cs, uint32_t cb, uint32_t *out)
{
    const uint32_t t = cs << 1;
    if (t <= F15) {
        *out = f_mul(F15 - f_mul(F15 - t, F15 - cb), cb);
    } else {
        uint32_t D;
        const uint32_t d4 = cb << 2;
        if (d4 <= F15) {
            const uint32_t sq = f_mul(cb, cb);
            D = d4 + 16 * f_mul(sq, cb) - 12 * sq;
        } else {
            D = f_sqrt(cb);
        }
        *out = cb + f_mul(2 * cs - F15, D - cb);
    }
}
static void b_softlight(uint32_t sr, uint32_t sg, uint32_t sb,
                        uint32_t *dr, uint32_t *dg, uint32_t *db)
{ softlight_channel(sr, *dr, dr); softlight_channel(sg, *dg, dg); softlight_channel(sb, *db, db); }
static void colorburn_channel(uint32_t cs, uint32_t cb, uint32_t *out)
{
    if (cs > 0) {
        const uint32_t t = f_div(F15 - cb, cs);
        *out = (t < F15) ? F15 - t : 0;
    } else *out = 0;
}
static void b_colorburn(uint32_t sr, uint32_t sg, uint32_t sb,
                        uint32_t *dr, uint32_t *dg, uint32_t *db)
{ colorburn_channel(sr, *dr, dr); colorburn_channel(sg, *dg, dg); colorburn_channel(sb, *db, db); }
static void colordodge_channel(uint32_t cs, uint32_t cb, uint32_t *out)
{
    if (cs < F15) {
        const uint32_t t = f_div(cb, F15 - cs);
        *out = (t < F15) ? t : F15;
    } else *out = F15;
}
static void b_colordodge(uint32_t sr, uint32_t sg, uint32_t sb,
                         uint32_t *dr, uint32_t *dg, uint32_t *db)
{ colordodge_channel(sr, *dr, dr); colordodge_channel(sg, *dg, dg); colordodge_channel(sb, *db, db); }
static void b_difference(uint32_t sr, uint32_t sg, uint32_t sb,
                         uint32_t *dr, uint32_t *dg, uint32_t *db)
{ *dr = sr >= *dr ? sr - *dr : *dr - sr; *dg = sg >= *dg ? sg - *dg : *dg - sg; *db = sb >= *db ? sb - *db : *db - sb; }
static void b_exclusion(uint32_t sr, uint32_t sg, uint32_t sb,
                        uint32_t *dr, uint32_t *dg, uint32_t *db)
{ *dr = *dr + sr - (f_mul(*dr, sr) << 1); *dg = *dg + sg - (f_mul(*dg, sg) << 1); *db = *db + sb - (f_mul(*db, sb) << 1); }

/* Nonseparable helpers (0.3 / 0.59 / 0.11 luminance) */
static const uint32_t LUM_R = (uint32_t)(0.3f * F15);
static const uint32_t LUM_G = (uint32_t)(0.59f * F15);
static const uint32_t LUM_B = (uint32_t)(0.11f * F15);
static uint32_t lum(uint32_t r, uint32_t g, uint32_t b)
{ return (r * LUM_R + g * LUM_G + b * LUM_B) / F15; }
static void clipcolor(uint32_t *r, uint32_t *g, uint32_t *b)
{
    int32_t ir = (int32_t)*r, ig = (int32_t)*g, ib = (int32_t)*b;
    const int32_t il = (int32_t)lum(*r, *g, *b);
    const int32_t imn = ir < ig ? (ir < ib ? ir : ib) : (ig < ib ? ig : ib);
    const int32_t imx = ir > ig ? (ir > ib ? ir : ib) : (ig > ib ? ig : ib);
    if (imn < 0) {
        const int32_t d = il - imn;
        ir = il + ((ir - il) * il) / d;
        ig = il + ((ig - il) * il) / d;
        ib = il + ((ib - il) * il) / d;
    }
    if (imx > (int32_t)F15) {
        const int32_t om = (int32_t)F15 - il;
        const int32_t d = imx - il;
        ir = il + ((ir - il) * om) / d;
        ig = il + ((ig - il) * om) / d;
        ib = il + ((ib - il) * om) / d;
    }
    *r = (uint32_t)ir; *g = (uint32_t)ig; *b = (uint32_t)ib;
}
static void setlum(uint32_t *r, uint32_t *g, uint32_t *b, uint32_t target)
{
    const int32_t diff = (int32_t)target - (int32_t)lum(*r, *g, *b);
    *r = (uint32_t)((int32_t)*r + diff);
    *g = (uint32_t)((int32_t)*g + diff);
    *b = (uint32_t)((int32_t)*b + diff);
    clipcolor(r, g, b);
}
static uint32_t sat(uint32_t r, uint32_t g, uint32_t b)
{
    const uint32_t mn = r < g ? (r < b ? r : b) : (g < b ? g : b);
    const uint32_t mx = r > g ? (r > b ? r : b) : (g > b ? g : b);
    return mx - mn;
}
static void setsat(uint32_t *r, uint32_t *g, uint32_t *b, uint32_t s)
{
    uint32_t *top = b, *mid = g, *bot = r, *tmp;
    if (*top < *mid) { tmp = top; top = mid; mid = tmp; }
    if (*top < *bot) { tmp = top; top = bot; bot = tmp; }
    if (*mid < *bot) { tmp = mid; mid = bot; bot = tmp; }
    if (*top > *bot) {
        *mid = (*mid - *bot) * s / (*top - *bot);
        *top = s;
    } else { *top = 0; *mid = 0; }
    *bot = 0;
}
static void b_hue(uint32_t sr, uint32_t sg, uint32_t sb,
                  uint32_t *dr, uint32_t *dg, uint32_t *db)
{
    const uint32_t dl = lum(*dr, *dg, *db);
    const uint32_t ds = sat(*dr, *dg, *db);
    uint32_t r = sr, g = sg, b = sb;
    setsat(&r, &g, &b, ds);
    setlum(&r, &g, &b, dl);
    *dr = r; *dg = g; *db = b;
}
static void b_saturation(uint32_t sr, uint32_t sg, uint32_t sb,
                         uint32_t *dr, uint32_t *dg, uint32_t *db)
{
    const uint32_t dl = lum(*dr, *dg, *db);
    const uint32_t ss = sat(sr, sg, sb);
    uint32_t r = *dr, g = *dg, b = *db;
    setsat(&r, &g, &b, ss);
    setlum(&r, &g, &b, dl);
    *dr = r; *dg = g; *db = b;
}
static void b_color(uint32_t sr, uint32_t sg, uint32_t sb,
                    uint32_t *dr, uint32_t *dg, uint32_t *db)
{
    const uint32_t dl = lum(*dr, *dg, *db);
    uint32_t r = sr, g = sg, b = sb;
    setlum(&r, &g, &b, dl);
    *dr = r; *dg = g; *db = b;
}
static void b_luminosity(uint32_t sr, uint32_t sg, uint32_t sb,
                         uint32_t *dr, uint32_t *dg, uint32_t *db)
{
    const uint32_t sl = lum(sr, sg, sb);
    uint32_t r = *dr, g = *dg, b = *db;
    setlum(&r, &g, &b, sl);
    *dr = r; *dg = g; *db = b;
}

/* MyPaint compositing.hpp: composite functors --------------------------- */
static void c_sourceover(uint32_t Rs, uint32_t Gs, uint32_t Bs, uint32_t as,
                         uint16_t *rb, uint16_t *gb, uint16_t *bb, uint16_t *ab)
{
    const uint32_t j = F15 - as;
    const uint32_t k = f_mul(*ab, j);
    *rb = (uint16_t)f_short_clamp(f_sumprods(as, Rs, j, *rb));
    *gb = (uint16_t)f_short_clamp(f_sumprods(as, Gs, j, *gb));
    *bb = (uint16_t)f_short_clamp(f_sumprods(as, Bs, j, *bb));
    *ab = (uint16_t)f_short_clamp(as + k);
}
static void c_lighter(uint32_t Rs, uint32_t Gs, uint32_t Bs, uint32_t as,
                      uint16_t *rb, uint16_t *gb, uint16_t *bb, uint16_t *ab)
{
    *rb = (uint16_t)f_short_clamp(f_mul(Rs, as) + *rb);
    *gb = (uint16_t)f_short_clamp(f_mul(Gs, as) + *gb);
    *bb = (uint16_t)f_short_clamp(f_mul(Bs, as) + *bb);
    *ab = (uint16_t)f_short_clamp(*ab + as);
}

/* The reference one-pixel combine, mirroring MyPaint's tile_combine /
 * combine_data dispatch (specialized premultiplied paths first, then the
 * generic blend + composite pipeline). dst is premultiplied in/out. */
static void ref_combine(int mode, const uint16_t src[4], uint16_t dst[4],
                        uint32_t opac)
{
    const uint32_t src_r = src[0], src_g = src[1], src_b = src[2], src_a = src[3];

    if (mode == 0) { /* Normal + svg:src-over (specialized premult path) */
        const uint32_t Sa = f_mul(src_a, opac);
        const uint32_t om = F15 - Sa;
        dst[0] = (uint16_t)f_short_clamp(f_sumprods(src_r, opac, om, dst[0]));
        dst[1] = (uint16_t)f_short_clamp(f_sumprods(src_g, opac, om, dst[1]));
        dst[2] = (uint16_t)f_short_clamp(f_sumprods(src_b, opac, om, dst[2]));
        dst[3] = (uint16_t)f_short_clamp(Sa + f_mul(dst[3], om));
        return;
    }
    if (mode == 21) { /* Normal + mypaint:spectral-wgm (specialized) */
        const uint32_t Sa = f_mul(src_a, opac);
        const uint32_t om = F15 - Sa;
        if (dst[3] == 0 || Sa == F15 || Sa == 0) {
            dst[0] = (uint16_t)f_short_clamp(f_sumprods(src_r, opac, om, dst[0]));
            dst[1] = (uint16_t)f_short_clamp(f_sumprods(src_g, opac, om, dst[1]));
            dst[2] = (uint16_t)f_short_clamp(f_sumprods(src_b, opac, om, dst[2]));
            dst[3] = (uint16_t)f_short_clamp(Sa + f_mul(dst[3], om));
        } else {
            const float fac_a = (float)Sa / (Sa + om * dst[3] / (float)F15);
            const float fac_b = 1.0f - fac_a;
            float spectral_b[10] = {0};
            rgb_to_spectral((float)dst[0] / dst[3], (float)dst[1] / dst[3],
                            (float)dst[2] / dst[3], spectral_b);
            float spectral_a[10] = {0};
            if (src_a > 0)
                rgb_to_spectral((float)src_r / src_a, (float)src_g / src_a,
                                (float)src_b / src_a, spectral_a);
            else
                rgb_to_spectral((float)src_r / F15, (float)src_g / F15,
                                (float)src_b / F15, spectral_a);
            float result[10] = {0};
            for (int i = 0; i < 10; i++)
                result[i] = ref_fastpow(spectral_a[i], fac_a) *
                            ref_fastpow(spectral_b[i], fac_b);
            float rgb[3] = {0};
            spectral_to_rgb(result, rgb);
            const uint32_t out_a = f_short_clamp(Sa + f_mul(dst[3], om));
            dst[0] = (uint16_t)(rgb[0] * (out_a + 0.5f));
            dst[1] = (uint16_t)(rgb[1] * (out_a + 0.5f));
            dst[2] = (uint16_t)(rgb[2] * (out_a + 0.5f));
            dst[3] = (uint16_t)out_a;
        }
        return;
    }
    if (mode == 17) { /* dst-in (specialized premult path) */
        const uint32_t Sa = f_mul(src_a, opac);
        dst[0] = (uint16_t)f_short_clamp(f_mul(dst[0], Sa));
        dst[1] = (uint16_t)f_short_clamp(f_mul(dst[1], Sa));
        dst[2] = (uint16_t)f_short_clamp(f_mul(dst[2], Sa));
        dst[3] = (uint16_t)f_short_clamp(f_mul(dst[3], Sa));
        return;
    }
    if (mode == 18) { /* dst-out (specialized premult path) */
        const uint32_t j = F15 - f_mul(src_a, opac);
        dst[0] = (uint16_t)f_short_clamp(f_mul(dst[0], j));
        dst[1] = (uint16_t)f_short_clamp(f_mul(dst[1], j));
        dst[2] = (uint16_t)f_short_clamp(f_mul(dst[2], j));
        dst[3] = (uint16_t)f_short_clamp(f_mul(dst[3], j));
        return;
    }
    if (mode == 19) { /* src-atop (specialized premult path) */
        const uint32_t as = f_mul(src_a, opac);
        const uint32_t ab = dst[3];
        const uint32_t oma = F15 - as;
        dst[0] = (uint16_t)f_short_clamp(f_sumprods(f_mul(src_r, opac), ab, dst[0], oma));
        dst[1] = (uint16_t)f_short_clamp(f_sumprods(f_mul(src_g, opac), ab, dst[1], oma));
        dst[2] = (uint16_t)f_short_clamp(f_sumprods(f_mul(src_b, opac), ab, dst[2], oma));
        return;
    }
    if (mode == 20) { /* dst-atop (specialized premult path) */
        const uint32_t as = f_mul(src_a, opac);
        const uint32_t omb = F15 - dst[3];
        dst[0] = (uint16_t)f_short_clamp(f_sumprods(f_mul(src_r, opac), omb, dst[0], as));
        dst[1] = (uint16_t)f_short_clamp(f_sumprods(f_mul(src_g, opac), omb, dst[1], as));
        dst[2] = (uint16_t)f_short_clamp(f_sumprods(f_mul(src_b, opac), omb, dst[2], as));
        dst[3] = (uint16_t)as;
        return;
    }

    /* Generic path: unpremultiply, blend, Co, composite. */
    uint32_t sr, sg, sb, ra, ga, ba;
    if (src_a == 0) { sr = sg = sb = 0; }
    else {
        sr = f_short_clamp(f_div(src_r, src_a));
        sg = f_short_clamp(f_div(src_g, src_a));
        sb = f_short_clamp(f_div(src_b, src_a));
    }
    const uint32_t ab = dst[3];
    if (ab == 0) { ra = ga = ba = 0; }
    else {
        ra = f_short_clamp(f_div(dst[0], ab));
        ga = f_short_clamp(f_div(dst[1], ab));
        ba = f_short_clamp(f_div(dst[2], ab));
    }
    static const blendfn BLEND[16] = {
        b_normal, b_multiply, b_screen, b_overlay, b_darken, b_lighten,
        b_hardlight, b_softlight, b_colorburn, b_colordodge, b_difference,
        b_exclusion, b_hue, b_saturation, b_color, b_luminosity
    };
    BLEND[mode == 16 ? 0 : mode](sr, sg, sb, &ra, &ga, &ba);
    const uint32_t omb = F15 - ab;
    ra = f_sumprods(omb, sr, ab, ra);
    ga = f_sumprods(omb, sg, ab, ga);
    ba = f_sumprods(omb, sb, ab, ba);
    const uint32_t aso = f_mul(src_a, opac);
    if (mode == 16) {
        c_lighter(ra, ga, ba, aso, &dst[0], &dst[1], &dst[2], &dst[3]);
    } else {
        c_sourceover(ra, ga, ba, aso, &dst[0], &dst[1], &dst[2], &dst[3]);
    }
}

static int failures = 0;
static int worst_delta[22] = {0};
static const char *const MODE_NAMES[] = {
    "src-over", "multiply", "screen", "overlay", "darken", "lighten",
    "hard-light", "soft-light", "color-burn", "color-dodge", "difference",
    "exclusion", "hue", "saturation", "color", "luminosity", "plus",
    "dst-in", "dst-out", "src-atop", "dst-atop", "spectral-wgm"
};

static void check_parity(int mode, const uint16_t src[4], uint16_t ours[4],
                         uint16_t ref[4])
{
    /* 21 modes are bit-exact (0 LSB); only Pigment's float fastpow ordering
     * costs up to 1 LSB. Keep the asserted tolerance at the measured bound. */
    const int tol = (mode == 21) ? 1 : 0;
    for (int c = 0; c < 4; c++) {
        int diff = (int)ours[c] - (int)ref[c];
        if (diff < 0) diff = -diff;
        if (diff > tol) {
            if (failures < 20) {
                printf("MODE %2d %-12s src(%4d,%4d,%4d,a%4d) ours(%4d,%4d,%4d,%4d) "
                       "ref(%4d,%4d,%4d,%4d) channel %d\n",
                       mode, MODE_NAMES[mode], src[0], src[1], src[2], src[3],
                       ours[0], ours[1], ours[2], ours[3],
                       ref[0], ref[1], ref[2], ref[3], c);
            }
            failures++;
        }
        if (diff > worst_delta[mode]) worst_delta[mode] = diff;
    }
}

static void run_grid(int mode)
{
    static const uint16_t LEVELS[] = {0, 4096, 8192, 12288, 16384, 20480, 24576, 28672, 32768};
    static const uint16_t ALPHAS[] = {0, 4096, 8192, 16384, 24576, 32768};
    static const float OPACS[] = {0.25f, 0.5f, 0.75f, 1.0f};
    static const uint16_t TRI[] = {
        0, 0, 0, 32768, 0, 0, 32768, 32768, 0, 0, 32768, 0,
        0, 0, 32768, 32768, 0, 32768, 16384, 8192, 24576,
        8192, 24576, 16384, 24576, 16384, 8192
    };
    const size_t ntri = sizeof(TRI) / sizeof(TRI[0]) / 3;
    const size_t na = sizeof(ALPHAS) / sizeof(ALPHAS[0]);
    const size_t nl = sizeof(LEVELS) / sizeof(LEVELS[0]);
    uint16_t src[4], dst[4], ours[4], ref[4];
    for (size_t sa = 0; sa < na; sa++)
    for (size_t da = 0; da < na; da++)
    for (size_t sl = 0; sl < nl; sl++)
    for (size_t dl = 0; dl < nl; dl++)
    for (size_t oi = 0; oi < 4; oi++) {
        uint16_t sr, sg, sb, dr, dg, db;
        if (mode >= 12 && mode <= 15) {
            sr = TRI[(sa % ntri) * 3]; sg = TRI[(sa % ntri) * 3 + 1]; sb = TRI[(sa % ntri) * 3 + 2];
            dr = TRI[(da % ntri) * 3]; dg = TRI[(da % ntri) * 3 + 1]; db = TRI[(da % ntri) * 3 + 2];
        } else {
            sr = sg = sb = LEVELS[sl];
            dr = dg = db = LEVELS[dl];
        }
        src[0] = (uint16_t)f_mul(sr, ALPHAS[sa]);
        src[1] = (uint16_t)f_mul(sg, ALPHAS[sa]);
        src[2] = (uint16_t)f_mul(sb, ALPHAS[sa]);
        src[3] = ALPHAS[sa];
        dst[0] = (uint16_t)f_mul(dr, ALPHAS[da]);
        dst[1] = (uint16_t)f_mul(dg, ALPHAS[da]);
        dst[2] = (uint16_t)f_mul(db, ALPHAS[da]);
        dst[3] = ALPHAS[da];
        ours[0] = dst[0]; ours[1] = dst[1]; ours[2] = dst[2]; ours[3] = dst[3];
        ref[0] = dst[0];  ref[1] = dst[1];  ref[2] = dst[2];  ref[3] = dst[3];
        ref_combine(mode, src, ref, (uint32_t)(OPACS[oi] * F15));
        afterglow_layer_blend_over(ours, src, OPACS[oi], mode);
        check_parity(mode, src, ours, ref);
    }
}

int main(void)
{
    for (int mode = 0; mode < 22; mode++) {
        run_grid(mode);
        printf("mode %2d %-12s worst-delta %d\n", mode, MODE_NAMES[mode],
               worst_delta[mode]);
    }
    if (failures > 0) {
        printf("PARITY FAILURES: %d\n", failures);
        return 1;
    }
    printf("PARITY OK: afterglow_layer_blend_over matches MyPaint's layer "
           "stack for all 22 modes.\n");
    return 0;
}
