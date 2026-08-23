#ifndef AFTERGLOW_MYPAINT_BRUSH_COOPERATIVE_H
#define AFTERGLOW_MYPAINT_BRUSH_COOPERATIVE_H

#include "mypaint-brush.h"
#include "mypaint-surface.h"

int afterglow_brush_stroke_start(MyPaintBrush *brush, MyPaintSurface *surface,
                                 float x, float y, float pressure,
                                 float xtilt, float ytilt, double dtime,
                                 float viewzoom, float viewrotation,
                                 float barrel_rotation, int linear,
                                 int dab_budget);
int afterglow_brush_stroke_continue(int dab_budget);
int afterglow_brush_stroke_pending(void);
void afterglow_brush_stroke_cancel(void);

#endif
