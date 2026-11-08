// CutOut engine C API. Platform-neutral: used by the macOS SwiftUI app and
// intended for the future Windows front end.
//
// Threading: a CutoutEngine must only be used from one thread at a time (the
// UI thread). CutoutModel functions are thread-safe and are meant to run on
// a background queue; cutout_model_cancel may be called from any thread.
//
// Memory: strings returned by the library are freed with cutout_string_free,
// buffers with cutout_buffer_free. CutoutFrame.data is borrowed and valid
// only until the next call into the same engine.

#ifndef CUTOUT_H
#define CUTOUT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct CutoutEngine CutoutEngine;
typedef struct CutoutModel CutoutModel;

typedef struct {
    const uint8_t *data;   // premultiplied RGBA, width*height*4
    uint32_t width, height;
    int32_t dirty_x, dirty_y, dirty_w, dirty_h; // region changed by this render
    uint64_t generation;
} CutoutFrame;

typedef struct {
    uint8_t *data;
    size_t len;            // bytes
    uint32_t width, height; // for outlines: width = number of floats
} CutoutBuffer;

CutoutEngine *cutout_engine_new(void);
void cutout_engine_free(CutoutEngine *engine);
void cutout_string_free(char *s);
void cutout_buffer_free(CutoutBuffer buffer);

// JSON command protocol, e.g. {"cmd":"magic_wand","x":10,"y":20,"tolerance":12}
char *cutout_command(CutoutEngine *engine, const char *json);

int64_t cutout_import_rgba(CutoutEngine *engine, const uint8_t *rgba, uint32_t width, uint32_t height,
                           const char *name, bool as_layer, char **error);
CutoutFrame cutout_render(CutoutEngine *engine);
CutoutBuffer cutout_export_rgba(CutoutEngine *engine, bool unmasked);
CutoutBuffer cutout_render_unmasked(CutoutEngine *engine);
CutoutBuffer cutout_layer_pixels(CutoutEngine *engine, int64_t layer_id);
CutoutBuffer cutout_layer_thumbnail(CutoutEngine *engine, int64_t layer_id, uint32_t max_size);
bool cutout_set_layer_mask(CutoutEngine *engine, int64_t layer_id, const uint8_t *alpha, uint32_t width,
                           uint32_t height, const char *undo_label, char **error);
CutoutBuffer cutout_selection_outline(CutoutEngine *engine);
// update_id > 0 updates that text layer; otherwise a new layer is placed at (x, y).
int64_t cutout_text_layer(CutoutEngine *engine, int64_t update_id, const char *props_json, const uint8_t *rgba,
                          uint32_t width, uint32_t height, double x, double y, char **error);

// accel: 0 = CPU, 1 = CoreML (Neural Engine / GPU)
CutoutModel *cutout_model_load(const char *path, int32_t accel, const char *cache_dir, char **error);
void cutout_model_free(CutoutModel *model);
void cutout_model_cancel(const CutoutModel *model);
CutoutBuffer cutout_model_predict(const CutoutModel *model, const uint8_t *rgba, uint32_t width, uint32_t height,
                                  bool refine, char **error);
CutoutBuffer cutout_refine_alpha(const uint8_t *rgba, const uint8_t *alpha, uint32_t width, uint32_t height,
                                 float radius);
// Core ML path helpers: the exact pre/post-processing of cutout_model_predict.
// preprocess writes 3*iw*ih floats (planar, ImageNet-normalised) into out.
bool cutout_ai_preprocess(const uint8_t *rgba, uint32_t width, uint32_t height, uint32_t iw, uint32_t ih, float *out);
// probs: iw*ih logits or probabilities; returns a width*height matte.
CutoutBuffer cutout_ai_postprocess(const float *probs, uint32_t iw, uint32_t ih, const uint8_t *rgba, uint32_t width,
                                   uint32_t height, bool refine);

#ifdef __cplusplus
}
#endif

#endif
