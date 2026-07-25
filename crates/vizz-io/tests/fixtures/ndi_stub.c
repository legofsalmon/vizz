// Stub implementing the subset of the NDI C ABI that vizz-io/src/ndi.rs
// calls. It lets CI verify the FFI layer — struct layout, field values,
// pixel data, and teardown order — without the proprietary NDI SDK, which
// is registration-walled and cannot be installed on a runner.
//
// Struct definitions here are written from Processing.NDI.structs.h /
// Processing.NDI.Send.h *independently* of the Rust side, so a layout
// mistake in either shows up as garbage in the log rather than silently
// agreeing.
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    const char *p_ndi_name;
    const char *p_groups;
    bool clock_video, clock_audio;
} send_create_t;

typedef struct {
    int xres, yres;
    int FourCC;
    int frame_rate_N, frame_rate_D;
    float picture_aspect_ratio;
    int frame_format_type;
    int64_t timecode;
    const uint8_t *p_data;
    int line_stride_in_bytes;
    const char *p_metadata;
    int64_t timestamp;
} video_frame_v2_t;

static FILE *out;

static void emit(void) {
    if (!out) {
        const char *path = getenv("NDI_STUB_LOG");
        out = path ? fopen(path, "w") : stderr;
        if (!out) out = stderr;
    }
}

bool NDIlib_initialize(void) {
    emit();
    fprintf(out, "initialize\n");
    fflush(out);
    return true;
}

void NDIlib_destroy(void) {
    emit();
    fprintf(out, "destroy\n");
    fflush(out);
}

void *NDIlib_send_create(const send_create_t *s) {
    emit();
    fprintf(out, "create name=%s groups=%s clock_video=%d clock_audio=%d\n",
            s->p_ndi_name ? s->p_ndi_name : "(null)",
            s->p_groups ? s->p_groups : "(null)",
            (int)s->clock_video, (int)s->clock_audio);
    fflush(out);
    return (void *)0xABCD;
}

void NDIlib_send_destroy(void *inst) {
    emit();
    fprintf(out, "send_destroy inst=%s\n", inst == (void *)0xABCD ? "ok" : "WRONG");
    fflush(out);
}

void NDIlib_send_send_video_v2(void *inst, const video_frame_v2_t *f) {
    emit();
    // Sample a center pixel to prove real rendered content arrived intact
    // at the advertised stride.
    const uint8_t *px = f->p_data + (f->yres / 2) * (size_t)f->line_stride_in_bytes
                        + (f->xres / 2) * 4;
    fprintf(out,
            "frame inst=%s %dx%d fourcc=0x%08X stride=%d fps=%d/%d fmt=%d par=%.1f "
            "timecode=%lld bgra=%02X%02X%02X%02X\n",
            inst == (void *)0xABCD ? "ok" : "WRONG", f->xres, f->yres,
            (unsigned)f->FourCC, f->line_stride_in_bytes, f->frame_rate_N,
            f->frame_rate_D, f->frame_format_type, f->picture_aspect_ratio,
            (long long)f->timecode, px[0], px[1], px[2], px[3]);
    fflush(out);
}
