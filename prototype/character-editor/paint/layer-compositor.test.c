#include "layer-compositor.h"

#include <assert.h>
#include <stdint.h>

static uint32_t mul15(uint32_t a, uint32_t b)
{
    return (a * b) >> 15;
}

static uint32_t sumprods15(uint32_t a1, uint32_t a2, uint32_t b1, uint32_t b2)
{
    return (a1 * a2 + b1 * b2) >> 15;
}

static void test_normal(void)
{
    const uint16_t source[4] = {12000, 6000, 3000, 20000};
    uint16_t result[4] = {7000, 10000, 3000, 24000};
    const uint32_t opacity = 24576;
    const uint32_t alpha = mul15(source[3], opacity);
    const uint32_t inverse = 32768 - alpha;
    afterglow_layer_blend_over(result, source, 0.75f, WEB_MODE_NORMAL);
    assert(result[0] == sumprods15(source[0], opacity, inverse, 7000));
    assert(result[1] == sumprods15(source[1], opacity, inverse, 10000));
    assert(result[2] == sumprods15(source[2], opacity, inverse, 3000));
    assert(result[3] == alpha + mul15(24000, inverse));
}

static void test_porter_duff(void)
{
    const uint16_t source[4] = {12000, 6000, 3000, 20000};
    const uint32_t opacity = 24576;
    const uint32_t alpha = mul15(source[3], opacity);
    uint16_t result[4] = {7000, 10000, 3000, 24000};
    afterglow_layer_blend_over(result, source, 0.75f, WEB_MODE_SOURCE_ATOP);
    assert(result[0] == sumprods15(mul15(source[0], opacity), 24000,
                                   7000, 32768 - alpha));
    assert(result[1] == sumprods15(mul15(source[1], opacity), 24000,
                                   10000, 32768 - alpha));
    assert(result[2] == sumprods15(mul15(source[2], opacity), 24000,
                                   3000, 32768 - alpha));
    assert(result[3] == 24000);

    result[0] = 7000;
    result[1] = 10000;
    result[2] = 3000;
    result[3] = 24000;
    afterglow_layer_blend_over(result, source, 0.75f, WEB_MODE_DESTINATION_ATOP);
    assert(result[0] == sumprods15(mul15(source[0], opacity), 32768 - 24000,
                                   7000, alpha));
    assert(result[1] == sumprods15(mul15(source[1], opacity), 32768 - 24000,
                                   10000, alpha));
    assert(result[2] == sumprods15(mul15(source[2], opacity), 32768 - 24000,
                                   3000, alpha));
    assert(result[3] == alpha);
}

static void test_mode_range(void)
{
    const uint16_t source[4] = {13000, 22000, 9000, 21000};
    for (int mode = 0; mode < WEB_MODE_COUNT; mode++) {
        uint16_t result[4] = {17000, 8000, 25000, 26000};
        afterglow_layer_blend_over(result, source, 0.8f, mode);
        for (int channel = 0; channel < 4; channel++) {
            assert(result[channel] <= 32768);
        }
    }
}

int main(void)
{
    test_normal();
    test_porter_duff();
    test_mode_range();
    return 0;
}
