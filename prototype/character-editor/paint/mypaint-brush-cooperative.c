#define mypaint_brush_stroke_to mypaint_brush_stroke_to_upstream
#include "vendor/libmypaint/mypaint-brush.c"
#undef mypaint_brush_stroke_to

#include "mypaint-brush-cooperative.h"

typedef struct {
    int active;
    int painted;
    MyPaintBrush *brush;
    MyPaintSurface *surface;
    float x;
    float y;
    float pressure;
    float tilt_ascension;
    float tilt_declination;
    float tilt_declinationx;
    float tilt_declinationy;
    float viewzoom;
    float viewrotation;
    float barrel_rotation;
    int linear;
    double dtime;
    double dtime_left;
    float dabs_moved;
} AfterglowBrushStroke;

static AfterglowBrushStroke stroke;

enum {
    AFTERGLOW_PAINT_UNKNOWN = 0,
    AFTERGLOW_PAINT_YES = 1,
    AFTERGLOW_PAINT_NO = 2,
};

int mypaint_brush_stroke_to(MyPaintBrush *brush, MyPaintSurface *surface,
                            float x, float y, float pressure,
                            float xtilt, float ytilt, double dtime,
                            float viewzoom, float viewrotation,
                            float barrel_rotation, gboolean linear)
{
    return mypaint_brush_stroke_to_upstream(
        brush, surface, x, y, pressure, xtilt, ytilt, dtime,
        viewzoom, viewrotation, barrel_rotation, linear);
}

void afterglow_brush_stroke_cancel(void)
{
    memset(&stroke, 0, sizeof(stroke));
}

int afterglow_brush_stroke_pending(void)
{
    return stroke.active;
}

static int finish_stroke(void)
{
    MyPaintBrush *self = stroke.brush;
    const float dabs_todo = count_dabs_to(
        self, stroke.x, stroke.y, stroke.dtime_left);
    float step_dpressure;
    const float step_ddab = dabs_todo;
    const float step_dx = stroke.x - STATE(self, X);
    const float step_dy = stroke.y - STATE(self, Y);
    step_dpressure = stroke.pressure - STATE(self, PRESSURE);
    const float step_declination = stroke.tilt_declination - STATE(self, DECLINATION);
    const float step_declinationx = stroke.tilt_declinationx - STATE(self, DECLINATIONX);
    const float step_declinationy = stroke.tilt_declinationy - STATE(self, DECLINATIONY);
    const float step_ascension = smallest_angular_difference(
        STATE(self, ASCENSION), stroke.tilt_ascension);
    const float step_dtime = (float)stroke.dtime_left;
    const float step_barrel_rotation = smallest_angular_difference(
        STATE(self, BARREL_ROTATION), stroke.barrel_rotation * 360.0f);

    update_states_and_setting_values(
        self, step_ddab, step_dx, step_dy, step_dpressure,
        step_declination, step_ascension, step_dtime, stroke.viewzoom,
        stroke.viewrotation, step_declinationx, step_declinationy,
        step_barrel_rotation);
    STATE(self, PARTIAL_DABS) = stroke.dabs_moved + dabs_todo;

    int split = 0;
    int painted = stroke.painted;
    if (painted == AFTERGLOW_PAINT_UNKNOWN) {
        if (self->stroke_current_idling_time > 0 ||
            self->stroke_total_painting_time == 0) {
            painted = AFTERGLOW_PAINT_NO;
        } else {
            painted = AFTERGLOW_PAINT_YES;
        }
    }
    if (painted == AFTERGLOW_PAINT_YES) {
        self->stroke_total_painting_time += stroke.dtime;
        self->stroke_current_idling_time = 0;
        if (self->stroke_total_painting_time > 4 + 3 * stroke.pressure &&
            step_dpressure >= 0) {
            split = 1;
        }
    } else {
        self->stroke_current_idling_time += stroke.dtime;
        if (self->stroke_total_painting_time == 0) {
            if (self->stroke_current_idling_time > 1.0) split = 1;
        } else if (self->stroke_total_painting_time +
                       self->stroke_current_idling_time >
                   0.9 + 5 * stroke.pressure) {
            split = 1;
        }
    }
    stroke.active = 0;
    return split ? 2 : 1;
}

int afterglow_brush_stroke_continue(int dab_budget)
{
    if (!stroke.active || !stroke.brush || !stroke.surface || dab_budget < 1) {
        return -1;
    }
    MyPaintBrush *self = stroke.brush;
    int processed = 0;
    float dabs_todo = count_dabs_to(
        self, stroke.x, stroke.y, stroke.dtime_left);
    if (!isfinite(dabs_todo)) {
        afterglow_brush_stroke_cancel();
        return -2;
    }

    while (stroke.dabs_moved + dabs_todo >= 1.0f &&
           processed < dab_budget) {
        float step_ddab;
        if (stroke.dabs_moved > 0) {
            step_ddab = 1.0f - stroke.dabs_moved;
            stroke.dabs_moved = 0;
        } else {
            step_ddab = 1.0f;
        }
        const float frac = step_ddab / dabs_todo;
        const float old_x = STATE(self, X);
        const float old_y = STATE(self, Y);
        const float old_pressure = STATE(self, PRESSURE);
        const double old_dtime_left = stroke.dtime_left;
        const float step_dx = frac * (stroke.x - old_x);
        const float step_dy = frac * (stroke.y - old_y);
        const float step_dpressure = frac * (stroke.pressure - old_pressure);
        const float step_dtime = frac * (stroke.dtime_left - 0.0);
        const float step_declination = frac *
            (stroke.tilt_declination - STATE(self, DECLINATION));
        const float step_declinationx = frac *
            (stroke.tilt_declinationx - STATE(self, DECLINATIONX));
        const float step_declinationy = frac *
            (stroke.tilt_declinationy - STATE(self, DECLINATIONY));
        const float step_ascension = frac * smallest_angular_difference(
            STATE(self, ASCENSION), stroke.tilt_ascension);
        const float step_barrel_rotation = frac * smallest_angular_difference(
            STATE(self, BARREL_ROTATION), stroke.barrel_rotation * 360.0f);

        update_states_and_setting_values(
            self, step_ddab, step_dx, step_dy, step_dpressure,
            step_declination, step_ascension, step_dtime, stroke.viewzoom,
            stroke.viewrotation, step_declinationx, step_declinationy,
            step_barrel_rotation);

        STATE(self, FLIP) *= -1;
        const gboolean painted_now = prepare_and_draw_dab(
            self, stroke.surface, stroke.linear);
        if (painted_now) {
            stroke.painted = AFTERGLOW_PAINT_YES;
        } else if (stroke.painted == AFTERGLOW_PAINT_UNKNOWN) {
            stroke.painted = AFTERGLOW_PAINT_NO;
        }
        self->random_input = rng_double_next(self->rng);
        stroke.dtime_left -= step_dtime;
        processed++;

        dabs_todo = count_dabs_to(
            self, stroke.x, stroke.y, stroke.dtime_left);
        if (!isfinite(dabs_todo)) {
            afterglow_brush_stroke_cancel();
            return -2;
        }
        if (stroke.dabs_moved + dabs_todo >= 1.0f &&
            STATE(self, X) == old_x && STATE(self, Y) == old_y &&
            STATE(self, PRESSURE) == old_pressure &&
            stroke.dtime_left == old_dtime_left) {
            afterglow_brush_stroke_cancel();
            return -2;
        }
    }

    if (stroke.dabs_moved + dabs_todo >= 1.0f) return 0;
    return finish_stroke();
}

int afterglow_brush_stroke_start(MyPaintBrush *self, MyPaintSurface *surface,
                                 float x, float y, float pressure,
                                 float xtilt, float ytilt, double dtime,
                                 float viewzoom, float viewrotation,
                                 float barrel_rotation, int linear,
                                 int dab_budget)
{
    const float max_dtime = 5.0f;
    if (!self || !surface || stroke.active || dab_budget < 1) return -1;

    float tilt_ascension = 0.0f;
    float tilt_declination = 90.0f;
    float tilt_declinationx = 90.0f;
    float tilt_declinationy = 90.0f;
    if (xtilt != 0 || ytilt != 0) {
        xtilt = CLAMP(xtilt, -1.0f, 1.0f);
        ytilt = CLAMP(ytilt, -1.0f, 1.0f);
        tilt_ascension = DEGREES(atan2(-xtilt, ytilt));
        const float rad = hypot(xtilt, ytilt);
        tilt_declination = 90.0f - rad * 60.0f;
        tilt_declinationx = xtilt * 60.0f;
        tilt_declinationy = ytilt * 60.0f;
    }

    if (pressure <= 0.0f) pressure = 0.0f;
    if (!isfinite(x) || !isfinite(y) || x > 1e10f || y > 1e10f ||
        x < -1e10f || y < -1e10f) {
        x = 0.0f;
        y = 0.0f;
        pressure = 0.0f;
        viewzoom = 0.0f;
        viewrotation = 0.0f;
        barrel_rotation = 0.0f;
    }
    if (dtime <= 0) dtime = 0.0001;

    if (dtime > 0.100 && pressure && STATE(self, PRESSURE) == 0) {
        mypaint_brush_stroke_to_upstream(
            self, surface, x, y, 0.0f, 90.0f, 0.0f, dtime - 0.0001,
            viewzoom, viewrotation, 0.0f, linear);
        dtime = 0.0001;
    }

    if (self->skip > 0.001) {
        const float dist = hypotf(self->skip_last_x - x,
                                  self->skip_last_y - y);
        self->skip_last_x = x;
        self->skip_last_y = y;
        self->skipped_dtime += dtime;
        self->skip -= dist;
        dtime = self->skipped_dtime;
        if (self->skip > 0.001 &&
            !(dtime > max_dtime || self->reset_requested)) {
            return 1;
        }
        self->skip = 0;
        self->skip_last_x = 0;
        self->skip_last_y = 0;
        self->skipped_dtime = 0;
    }

    if (BASEVAL(self, TRACKING_NOISE)) {
        const float base_radius = expf(BASEVAL(self, RADIUS_LOGARITHMIC));
        const float noise = base_radius * BASEVAL(self, TRACKING_NOISE);
        if (noise > 0.001) {
            self->skip = 0.5f * noise;
            self->skip_last_x = x;
            self->skip_last_y = y;
            x += noise * rand_gauss(self->rng);
            y += noise * rand_gauss(self->rng);
        }
    }
    const float fac = 1.0f - exp_decay(
        BASEVAL(self, SLOW_TRACKING), 100.0f * dtime);
    x = STATE(self, X) + (x - STATE(self, X)) * fac;
    y = STATE(self, Y) + (y - STATE(self, Y)) * fac;

    if (dtime > max_dtime || self->reset_requested) {
        self->reset_requested = FALSE;
        brush_reset(self);
        self->random_input = rng_double_next(self->rng);
        STATE(self, X) = x;
        STATE(self, Y) = y;
        STATE(self, PRESSURE) = pressure;
        STATE(self, ACTUAL_X) = x;
        STATE(self, ACTUAL_Y) = y;
        STATE(self, STROKE) = 1.0f;
        return 2;
    }

    stroke.active = 1;
    stroke.painted = AFTERGLOW_PAINT_UNKNOWN;
    stroke.brush = self;
    stroke.surface = surface;
    stroke.x = x;
    stroke.y = y;
    stroke.pressure = pressure;
    stroke.tilt_ascension = tilt_ascension;
    stroke.tilt_declination = tilt_declination;
    stroke.tilt_declinationx = tilt_declinationx;
    stroke.tilt_declinationy = tilt_declinationy;
    stroke.viewzoom = viewzoom;
    stroke.viewrotation = viewrotation;
    stroke.barrel_rotation = barrel_rotation;
    stroke.linear = linear;
    stroke.dtime = dtime;
    stroke.dtime_left = dtime;
    stroke.dabs_moved = STATE(self, PARTIAL_DABS);
    return afterglow_brush_stroke_continue(dab_budget);
}
