#include <jni.h>
#include <stdint.h>
#include <string.h>
#include "novel_reader_probe.h"

static nr_engine *as_engine(jlong handle) {
    return (nr_engine *)(uintptr_t)handle;
}

static jbyteArray copy_and_release(JNIEnv *env, nr_buffer *buffer) {
    if (buffer == NULL) return NULL;
    jbyteArray result = (*env)->NewByteArray(env, (jsize)buffer->len);
    if (result != NULL && buffer->len > 0) {
        (*env)->SetByteArrayRegion(env, result, 0, (jsize)buffer->len,
                                   (const jbyte *)buffer->ptr);
    }
    /* Rust owns the allocation; release immediately after the Java copy. */
    nr_release_buffer(*buffer);
    memset(buffer, 0, sizeof(*buffer));
    return result;
}

static void noop_event(void *user, const uint8_t *json_utf8, size_t json_len) {
    (void)user;
    (void)json_utf8;
    (void)json_len;
}

JNIEXPORT jlong JNICALL
Java_com_novelreader_probe_core_NrCore_nativeCreate(JNIEnv *env, jobject self, jlong max_window_bytes) {
    (void)env; (void)self;
    return (jlong)(uintptr_t)nr_engine_create((uint64_t)max_window_bytes);
}

JNIEXPORT void JNICALL
Java_com_novelreader_probe_core_NrCore_nativeDestroy(JNIEnv *env, jobject self, jlong handle) {
    (void)env; (void)self;
    nr_engine_destroy(as_engine(handle));
}

JNIEXPORT jint JNICALL
Java_com_novelreader_probe_core_NrCore_nativeOpenBook(JNIEnv *env, jobject self, jlong handle,
                                                       jlong book_id, jbyteArray path,
                                                       jlong total_bytes) {
    (void)self;
    jbyte *path_bytes = NULL;
    jsize path_len = 0;
    if (path != NULL) {
        path_len = (*env)->GetArrayLength(env, path);
        path_bytes = (*env)->GetByteArrayElements(env, path, NULL);
    }
    int32_t code = nr_open_book(as_engine(handle), (uint64_t)book_id,
                                (const uint8_t *)path_bytes, (size_t)path_len,
                                (uint64_t)total_bytes);
    if (path_bytes != NULL) (*env)->ReleaseByteArrayElements(env, path, path_bytes, JNI_ABORT);
    return (jint)code;
}

JNIEXPORT jint JNICALL
Java_com_novelreader_probe_core_NrCore_nativeOpenExternalBook(JNIEnv *env, jobject self,
                                                              jlong handle, jlong book_id,
                                                              jlong total_bytes) {
    (void)env; (void)self;
    return (jint)nr_open_external_book(as_engine(handle), (uint64_t)book_id,
                                       (uint64_t)total_bytes);
}

JNIEXPORT jbyteArray JNICALL
Java_com_novelreader_probe_core_NrCore_nativeReadWindow(JNIEnv *env, jobject self, jlong handle,
                                                        jint chapter_index, jlong offset,
                                                        jint length) {
    (void)self;
    nr_buffer out = {0};
    int32_t code = nr_read_window(as_engine(handle), (uint32_t)chapter_index,
                                  (uint64_t)offset, (size_t)length, &out);
    if (code != NR_ERR_OK) return NULL;
    return copy_and_release(env, &out);
}

JNIEXPORT jint JNICALL
Java_com_novelreader_probe_core_NrCore_nativeReleaseBuffer(JNIEnv *env, jobject self) {
    (void)env; (void)self;
    nr_buffer empty = {0};
    return (jint)nr_release_buffer(empty);
}

JNIEXPORT jbyteArray JNICALL
Java_com_novelreader_probe_core_NrCore_nativeGetBookBatch(JNIEnv *env, jobject self, jlong handle,
                                                          jlong start, jint count) {
    (void)self;
    nr_buffer out = {0};
    int32_t code = nr_get_book_batch(as_engine(handle), (uint64_t)start, (uint32_t)count, &out);
    if (code != NR_ERR_OK) return NULL;
    return copy_and_release(env, &out);
}

JNIEXPORT jbyteArray JNICALL
Java_com_novelreader_probe_core_NrCore_nativeGetChapterBatch(JNIEnv *env, jobject self, jlong handle,
                                                             jlong book_id, jint start, jint count) {
    (void)self;
    nr_buffer out = {0};
    int32_t code = nr_get_chapter_batch(as_engine(handle), (uint64_t)book_id,
                                        (uint32_t)start, (uint32_t)count, &out);
    if (code != NR_ERR_OK) return NULL;
    return copy_and_release(env, &out);
}

JNIEXPORT jint JNICALL
Java_com_novelreader_probe_core_NrCore_nativeSavePosition(JNIEnv *env, jobject self, jlong handle,
                                                          jlong book_id, jint chapter_index,
                                                          jlong byte_offset) {
    (void)env; (void)self;
    return (jint)nr_save_position(as_engine(handle), (uint64_t)book_id,
                                  (uint32_t)chapter_index, (uint64_t)byte_offset);
}

JNIEXPORT jint JNICALL
Java_com_novelreader_probe_core_NrCore_nativeCommitPageBoundary(JNIEnv *env, jobject self,
                                                                jlong handle, jint chapter_index,
                                                                jlong start_byte, jlong end_byte,
                                                                jlong layout_id, jint line_count) {
    (void)env; (void)self;
    return (jint)nr_commit_page_boundary(as_engine(handle), (uint32_t)chapter_index,
                                         (uint64_t)start_byte, (uint64_t)end_byte,
                                         (uint64_t)layout_id, (uint32_t)line_count);
}

JNIEXPORT jint JNICALL
Java_com_novelreader_probe_core_NrCore_nativeInvalidateLayout(JNIEnv *env, jobject self,
                                                              jlong handle, jlong layout_id) {
    (void)env; (void)self;
    return (jint)nr_invalidate_layout(as_engine(handle), (uint64_t)layout_id, NULL);
}

JNIEXPORT jlong JNICALL
Java_com_novelreader_probe_core_NrCore_nativeSubscribeEvents(JNIEnv *env, jobject self, jlong handle) {
    (void)env; (void)self;
    return (jlong)nr_subscribe_events(as_engine(handle), noop_event, NULL);
}

JNIEXPORT jint JNICALL
Java_com_novelreader_probe_core_NrCore_nativeCancelRequest(JNIEnv *env, jobject self,
                                                           jlong handle, jlong request_id) {
    (void)env; (void)self;
    return (jint)nr_cancel_request(as_engine(handle), (uint64_t)request_id);
}
