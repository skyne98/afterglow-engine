#ifndef AFTERGLOW_LAYER_COMPOSITOR_H
#define AFTERGLOW_LAYER_COMPOSITOR_H

#include <stdint.h>

#define WEB_MODE_NORMAL 0
#define WEB_MODE_MULTIPLY 1
#define WEB_MODE_SCREEN 2
#define WEB_MODE_OVERLAY 3
#define WEB_MODE_DARKEN 4
#define WEB_MODE_LIGHTEN 5
#define WEB_MODE_HARD_LIGHT 6
#define WEB_MODE_SOFT_LIGHT 7
#define WEB_MODE_COLOR_BURN 8
#define WEB_MODE_COLOR_DODGE 9
#define WEB_MODE_DIFFERENCE 10
#define WEB_MODE_EXCLUSION 11
#define WEB_MODE_HUE 12
#define WEB_MODE_SATURATION 13
#define WEB_MODE_COLOR 14
#define WEB_MODE_LUMINOSITY 15
#define WEB_MODE_PLUS 16
#define WEB_MODE_DESTINATION_IN 17
#define WEB_MODE_DESTINATION_OUT 18
#define WEB_MODE_SOURCE_ATOP 19
#define WEB_MODE_DESTINATION_ATOP 20
#define WEB_MODE_PIGMENT 21
#define WEB_MODE_COUNT 22

void afterglow_layer_blend_over(uint16_t *dst, const uint16_t *src,
                                float opacity, int mode);

#endif
