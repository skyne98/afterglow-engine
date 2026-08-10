#include "layer-compositor.h"

#include <math.h>
#include <stdint.h>

#include "helpers.h"
#include "fastapprox/fastpow.h"

#define U15_ONE 32768u

typedef uint32_t u15_t;
typedef int32_t i15_t;

static u15_t u15_clamp(i15_t value)
{
    if (value <= 0) return 0;
    if ((u15_t)value >= U15_ONE) return U15_ONE;
    return (u15_t)value;
}

static u15_t u15_mul(u15_t a, u15_t b)
{
    return (a * b) >> 15;
}

static u15_t u15_sumprods(u15_t a1, u15_t a2, u15_t b1, u15_t b2)
{
    return (a1 * a2 + b1 * b2) >> 15;
}

static u15_t u15_div(u15_t a, u15_t b)
{
    return b == 0 ? U15_ONE : (a << 15) / b;
}

static u15_t u15_opacity(float opacity)
{
    if (!isfinite(opacity) || opacity <= 0.0f) return 0;
    if (opacity >= 1.0f) return U15_ONE;
    return (u15_t)(opacity * (float)U15_ONE);
}

static u15_t u15_sqrt(u15_t value)
{
    uint64_t target = (uint64_t)value * U15_ONE;
    uint64_t low = 0;
    uint64_t high = U15_ONE;
    while (low < high) {
        uint64_t middle = (low + high + 1) >> 1;
        if (middle * middle <= target) low = middle;
        else high = middle - 1;
    }
    return (u15_t)low;
}

static u15_t blend_channel(u15_t source, u15_t backdrop, int mode)
{
    switch (mode) {
    case WEB_MODE_NORMAL:
        return source;
    case WEB_MODE_MULTIPLY:
        return u15_mul(source, backdrop);
    case WEB_MODE_SCREEN:
        return backdrop + source - u15_mul(backdrop, source);
    case WEB_MODE_OVERLAY: {
        const u15_t two_backdrop = backdrop << 1;
        if (two_backdrop <= U15_ONE) return u15_mul(source, two_backdrop);
        const u15_t remainder = two_backdrop - U15_ONE;
        return source + remainder - u15_mul(source, remainder);
    }
    case WEB_MODE_DARKEN:
        return source < backdrop ? source : backdrop;
    case WEB_MODE_LIGHTEN:
        return source > backdrop ? source : backdrop;
    case WEB_MODE_HARD_LIGHT: {
        const u15_t two_source = source << 1;
        if (two_source <= U15_ONE) return u15_mul(backdrop, two_source);
        const u15_t remainder = two_source - U15_ONE;
        return backdrop + remainder - u15_mul(backdrop, remainder);
    }
    case WEB_MODE_SOFT_LIGHT: {
        const u15_t two_source = source << 1;
        u15_t result;
        if (two_source <= U15_ONE) {
            result = U15_ONE - u15_mul(U15_ONE - two_source,
                                      U15_ONE - backdrop);
            return u15_mul(result, backdrop);
        }
        const u15_t four_backdrop = backdrop << 2;
        u15_t d;
        if (four_backdrop <= U15_ONE) {
            const u15_t backdrop_squared = u15_mul(backdrop, backdrop);
            d = four_backdrop + 16 * u15_mul(backdrop_squared, backdrop)
                - 12 * backdrop_squared;
        } else {
            d = u15_sqrt(backdrop);
        }
        result = backdrop + u15_mul(two_source - U15_ONE, d - backdrop);
        return result;
    }
    case WEB_MODE_COLOR_BURN:
        if (source > 0) {
            const u15_t value = u15_div(U15_ONE - backdrop, source);
            if (value < U15_ONE) return U15_ONE - value;
        }
        return 0;
    case WEB_MODE_COLOR_DODGE:
        if (source < U15_ONE) {
            const u15_t value = u15_div(backdrop, U15_ONE - source);
            if (value < U15_ONE) return value;
        }
        return U15_ONE;
    case WEB_MODE_DIFFERENCE:
        return source >= backdrop ? source - backdrop : backdrop - source;
    case WEB_MODE_EXCLUSION:
        return backdrop + source - (u15_mul(backdrop, source) << 1);
    default:
        return source;
    }
}

static i15_t channel_min(i15_t r, i15_t g, i15_t b)
{
    i15_t result = r < g ? r : g;
    return result < b ? result : b;
}

static i15_t channel_max(i15_t r, i15_t g, i15_t b)
{
    i15_t result = r > g ? r : g;
    return result > b ? result : b;
}

static i15_t luminance(i15_t r, i15_t g, i15_t b)
{
    const uint32_t lum_r = 9830;
    const uint32_t lum_g = 19333;
    const uint32_t lum_b = 3604;
    return (r * lum_r + g * lum_g + b * lum_b) / U15_ONE;
}

static void clip_color(i15_t *r, i15_t *g, i15_t *b)
{
    const i15_t lum = luminance(*r, *g, *b);
    const i15_t minimum = channel_min(*r, *g, *b);
    const i15_t maximum = channel_max(*r, *g, *b);
    if (minimum < 0) {
        const i15_t denominator = lum - minimum;
        *r = lum + ((*r - lum) * lum) / denominator;
        *g = lum + ((*g - lum) * lum) / denominator;
        *b = lum + ((*b - lum) * lum) / denominator;
    }
    if (maximum > (i15_t)U15_ONE) {
        const i15_t denominator = maximum - lum;
        const i15_t one_minus_lum = U15_ONE - lum;
        *r = lum + ((*r - lum) * one_minus_lum) / denominator;
        *g = lum + ((*g - lum) * one_minus_lum) / denominator;
        *b = lum + ((*b - lum) * one_minus_lum) / denominator;
    }
}

static void set_luminance(i15_t *r, i15_t *g, i15_t *b, i15_t lum)
{
    const i15_t difference = lum - luminance(*r, *g, *b);
    *r += difference;
    *g += difference;
    *b += difference;
    clip_color(r, g, b);
}

static i15_t saturation(i15_t r, i15_t g, i15_t b)
{
    return channel_max(r, g, b) - channel_min(r, g, b);
}

static void set_saturation(i15_t *r, i15_t *g, i15_t *b, i15_t sat)
{
    i15_t *top = b;
    i15_t *middle = g;
    i15_t *bottom = r;
    i15_t *swap;
    if (*top < *middle) {
        swap = top;
        top = middle;
        middle = swap;
    }
    if (*top < *bottom) {
        swap = top;
        top = bottom;
        bottom = swap;
    }
    if (*middle < *bottom) {
        swap = middle;
        middle = bottom;
        bottom = swap;
    }
    if (*top > *bottom) {
        *middle = (*middle - *bottom) * sat / (*top - *bottom);
        *top = sat;
    } else {
        *top = 0;
        *middle = 0;
    }
    *bottom = 0;
}

static void nonseparable_color(u15_t source_r, u15_t source_g, u15_t source_b,
                               u15_t backdrop_r, u15_t backdrop_g,
                               u15_t backdrop_b, int mode,
                               u15_t *out_r, u15_t *out_g, u15_t *out_b)
{
    i15_t r;
    i15_t g;
    i15_t b;
    const i15_t backdrop_lum = luminance(backdrop_r, backdrop_g, backdrop_b);
    switch (mode) {
    case WEB_MODE_HUE:
        r = source_r;
        g = source_g;
        b = source_b;
        set_saturation(&r, &g, &b,
                       saturation(backdrop_r, backdrop_g, backdrop_b));
        set_luminance(&r, &g, &b, backdrop_lum);
        break;
    case WEB_MODE_SATURATION:
        r = backdrop_r;
        g = backdrop_g;
        b = backdrop_b;
        set_saturation(&r, &g, &b, saturation(source_r, source_g, source_b));
        set_luminance(&r, &g, &b, backdrop_lum);
        break;
    case WEB_MODE_COLOR:
        r = source_r;
        g = source_g;
        b = source_b;
        set_luminance(&r, &g, &b, backdrop_lum);
        break;
    default:
        r = backdrop_r;
        g = backdrop_g;
        b = backdrop_b;
        set_luminance(&r, &g, &b, luminance(source_r, source_g, source_b));
        break;
    }
    *out_r = u15_clamp(r);
    *out_g = u15_clamp(g);
    *out_b = u15_clamp(b);
}

static void pigment_blend(uint16_t *dst, const uint16_t *src,
                          u15_t source_alpha, u15_t opacity)
{
    const u15_t backdrop_alpha = dst[3];
    const u15_t one_minus_source = U15_ONE - source_alpha;
    if (backdrop_alpha == 0 || source_alpha == 0 || source_alpha == U15_ONE) {
        dst[0] = u15_clamp((i15_t)u15_sumprods(src[0], opacity,
                                                one_minus_source, dst[0]));
        dst[1] = u15_clamp((i15_t)u15_sumprods(src[1], opacity,
                                                one_minus_source, dst[1]));
        dst[2] = u15_clamp((i15_t)u15_sumprods(src[2], opacity,
                                                one_minus_source, dst[2]));
        dst[3] = u15_clamp((i15_t)(source_alpha + u15_mul(backdrop_alpha,
                                                           one_minus_source)));
        return;
    }

    const float denominator = (float)(source_alpha +
                                      (one_minus_source * backdrop_alpha) / U15_ONE);
    const float source_factor = (float)source_alpha / denominator;
    const float backdrop_factor = 1.0f - source_factor;
    float source_spectral[10] = {0};
    float backdrop_spectral[10] = {0};
    rgb_to_spectral((float)src[0] / src[3], (float)src[1] / src[3],
                    (float)src[2] / src[3], source_spectral);
    rgb_to_spectral((float)dst[0] / dst[3], (float)dst[1] / dst[3],
                    (float)dst[2] / dst[3], backdrop_spectral);
    float result_spectral[10] = {0};
    for (int i = 0; i < 10; i++) {
        result_spectral[i] = fastpow(source_spectral[i], source_factor) *
                             fastpow(backdrop_spectral[i], backdrop_factor);
    }
    float rgb[3] = {0};
    spectral_to_rgb(result_spectral, rgb);
    const u15_t out_alpha = u15_clamp((i15_t)(source_alpha +
                                                u15_mul(backdrop_alpha,
                                                        one_minus_source)));
    dst[0] = (uint16_t)(rgb[0] * (out_alpha + 0.5f));
    dst[1] = (uint16_t)(rgb[1] * (out_alpha + 0.5f));
    dst[2] = (uint16_t)(rgb[2] * (out_alpha + 0.5f));
    dst[3] = out_alpha;
    (void)opacity;
}

void afterglow_layer_blend_over(uint16_t *dst, const uint16_t *src,
                                float opacity, int mode)
{
    const u15_t source_opacity = u15_opacity(opacity);
    const u15_t source_alpha = u15_mul(src[3], source_opacity);
    const u15_t backdrop_alpha = dst[3];
    const u15_t source_r = src[3] ? u15_clamp((i15_t)u15_div(src[0], src[3])) : 0;
    const u15_t source_g = src[3] ? u15_clamp((i15_t)u15_div(src[1], src[3])) : 0;
    const u15_t source_b = src[3] ? u15_clamp((i15_t)u15_div(src[2], src[3])) : 0;
    const u15_t backdrop_r = dst[3] ? u15_clamp((i15_t)u15_div(dst[0], dst[3])) : 0;
    const u15_t backdrop_g = dst[3] ? u15_clamp((i15_t)u15_div(dst[1], dst[3])) : 0;
    const u15_t backdrop_b = dst[3] ? u15_clamp((i15_t)u15_div(dst[2], dst[3])) : 0;
    const u15_t one_minus_source = U15_ONE - source_alpha;

    if (mode == WEB_MODE_PIGMENT) {
        pigment_blend(dst, src, source_alpha, source_opacity);
        return;
    }
    if (mode == WEB_MODE_NORMAL) {
        dst[0] = u15_clamp((i15_t)u15_sumprods(src[0], source_opacity,
                                                one_minus_source, dst[0]));
        dst[1] = u15_clamp((i15_t)u15_sumprods(src[1], source_opacity,
                                                one_minus_source, dst[1]));
        dst[2] = u15_clamp((i15_t)u15_sumprods(src[2], source_opacity,
                                                one_minus_source, dst[2]));
        dst[3] = u15_clamp((i15_t)(source_alpha +
                                   u15_mul(backdrop_alpha, one_minus_source)));
        return;
    }
    if (mode == WEB_MODE_PLUS) {
        dst[0] = u15_clamp((i15_t)(u15_mul(source_r, source_alpha) + dst[0]));
        dst[1] = u15_clamp((i15_t)(u15_mul(source_g, source_alpha) + dst[1]));
        dst[2] = u15_clamp((i15_t)(u15_mul(source_b, source_alpha) + dst[2]));
        dst[3] = u15_clamp((i15_t)(backdrop_alpha + source_alpha));
        return;
    }
    if (mode == WEB_MODE_DESTINATION_IN || mode == WEB_MODE_DESTINATION_OUT) {
        const u15_t factor = mode == WEB_MODE_DESTINATION_IN
                           ? source_alpha : one_minus_source;
        dst[0] = u15_mul(dst[0], factor);
        dst[1] = u15_mul(dst[1], factor);
        dst[2] = u15_mul(dst[2], factor);
        dst[3] = u15_mul(dst[3], factor);
        return;
    }
    if (mode == WEB_MODE_SOURCE_ATOP) {
        const u15_t source_red = u15_mul(src[0], source_opacity);
        const u15_t source_green = u15_mul(src[1], source_opacity);
        const u15_t source_blue = u15_mul(src[2], source_opacity);
        dst[0] = u15_clamp((i15_t)u15_sumprods(source_red, backdrop_alpha,
                                                dst[0], one_minus_source));
        dst[1] = u15_clamp((i15_t)u15_sumprods(source_green, backdrop_alpha,
                                                dst[1], one_minus_source));
        dst[2] = u15_clamp((i15_t)u15_sumprods(source_blue, backdrop_alpha,
                                                dst[2], one_minus_source));
        return;
    }
    if (mode == WEB_MODE_DESTINATION_ATOP) {
        const u15_t source_red = u15_mul(src[0], source_opacity);
        const u15_t source_green = u15_mul(src[1], source_opacity);
        const u15_t source_blue = u15_mul(src[2], source_opacity);
        const u15_t one_minus_backdrop = U15_ONE - backdrop_alpha;
        dst[0] = u15_clamp((i15_t)u15_sumprods(source_red, one_minus_backdrop,
                                                dst[0], source_alpha));
        dst[1] = u15_clamp((i15_t)u15_sumprods(source_green, one_minus_backdrop,
                                                dst[1], source_alpha));
        dst[2] = u15_clamp((i15_t)u15_sumprods(source_blue, one_minus_backdrop,
                                                dst[2], source_alpha));
        dst[3] = source_alpha;
        return;
    }

    u15_t blend_r = source_r;
    u15_t blend_g = source_g;
    u15_t blend_b = source_b;
    if (mode >= WEB_MODE_HUE && mode <= WEB_MODE_LUMINOSITY) {
        nonseparable_color(source_r, source_g, source_b,
                           backdrop_r, backdrop_g, backdrop_b, mode,
                           &blend_r, &blend_g, &blend_b);
    } else {
        blend_r = blend_channel(source_r, backdrop_r, mode);
        blend_g = blend_channel(source_g, backdrop_g, mode);
        blend_b = blend_channel(source_b, backdrop_b, mode);
    }

    const u15_t one_minus_backdrop = U15_ONE - backdrop_alpha;
    const u15_t composite_r = u15_sumprods(one_minus_backdrop, source_r,
                                           backdrop_alpha, blend_r);
    const u15_t composite_g = u15_sumprods(one_minus_backdrop, source_g,
                                           backdrop_alpha, blend_g);
    const u15_t composite_b = u15_sumprods(one_minus_backdrop, source_b,
                                           backdrop_alpha, blend_b);
    dst[0] = u15_clamp((i15_t)u15_sumprods(source_alpha, composite_r,
                                            one_minus_source, dst[0]));
    dst[1] = u15_clamp((i15_t)u15_sumprods(source_alpha, composite_g,
                                            one_minus_source, dst[1]));
    dst[2] = u15_clamp((i15_t)u15_sumprods(source_alpha, composite_b,
                                            one_minus_source, dst[2]));
    dst[3] = u15_clamp((i15_t)(source_alpha +
                               u15_mul(backdrop_alpha, one_minus_source)));
}
