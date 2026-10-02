#pragma once
#include <stdint.h>

typedef struct {
  uint32_t version, multiple, joints, speed, tessellation, use_seed, seed;
  const char *texture_path;
} XPPipesOptions;
void *xp_pipes_create(void *context, const XPPipesOptions *options,
                      uint32_t width, uint32_t height);
int xp_pipes_frame(void *handle, uint32_t width, uint32_t height,
                   uint32_t pixel_width, uint32_t pixel_height);
void xp_pipes_destroy(void *handle);
