// Test APK only. No Rust runtime, network, store, or production app data.
#include <android/input.h>
#include <android/looper.h>
#include <android/native_activity.h>
#include <jni.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>

static atomic_int running;
static atomic_int motions;
static atomic_int text_reports;
static pthread_t reader;

static void *read_input(void *queue) {
    ALooper *looper = ALooper_prepare(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS);
    AInputQueue_attachLooper(queue, looper, 1, NULL, NULL);
    while (atomic_load(&running)) {
        if (ALooper_pollOnce(50, NULL, NULL, NULL) != 1) continue;
        AInputEvent *event;
        while (AInputQueue_getEvent(queue, &event) >= 0) {
            if (AInputQueue_preDispatchEvent(queue, event)) continue;
            if (AInputEvent_getType(event) == AINPUT_EVENT_TYPE_MOTION) {
                atomic_fetch_add(&motions, 1);
            }
            AInputQueue_finishEvent(queue, event, 1);
        }
    }
    AInputQueue_detachLooper(queue);
    return NULL;
}

static void input_created(ANativeActivity *activity, AInputQueue *queue) {
    atomic_store(&running, 1);
    pthread_create(&reader, NULL, read_input, queue);
}

static void input_destroyed(ANativeActivity *activity, AInputQueue *queue) {
    atomic_store(&running, 0);
    pthread_join(reader, NULL);
}

static void window_created(ANativeActivity *activity, ANativeWindow *window) {
    ANativeWindow_setBuffersGeometry(window, 0, 0, WINDOW_FORMAT_RGBA_8888);
    ANativeWindow_Buffer buffer;
    if (ANativeWindow_lock(window, &buffer, NULL) == 0) {
        for (int y = 0; y < buffer.height; y++) {
            for (int x = 0; x < buffer.width; x++) {
                ((uint32_t *)buffer.bits)[y * buffer.stride + x] = 0xff201810;
            }
        }
        ANativeWindow_unlockAndPost(window);
    }
}

void ANativeActivity_onCreate(ANativeActivity *activity, void *state, size_t size) {
    activity->callbacks->onNativeWindowCreated = window_created;
    activity->callbacks->onInputQueueCreated = input_created;
    activity->callbacks->onInputQueueDestroyed = input_destroyed;
}

JNIEXPORT jint JNICALL Java_io_github_xpjb_chad_test_InputBridgeTest_reports(
    JNIEnv *env, jclass type) {
    return atomic_load(&text_reports);
}

JNIEXPORT jint JNICALL Java_io_github_xpjb_chad_test_InputBridgeTest_touches(
    JNIEnv *env, jclass type) {
    return atomic_load(&motions);
}

JNIEXPORT void JNICALL Java_io_github_xpjb_chad_ChadActivity_nativeViewport(
    JNIEnv *env, jclass type, jint left, jint top, jint right, jint bottom,
    jboolean keyboard_visible) {}

JNIEXPORT void JNICALL Java_io_github_xpjb_chad_ChadActivity_nativeTextInput(
    JNIEnv *env, jclass type, jlong id, jlong generation, jlong document_start,
    jstring text, jint selection_start, jint selection_end,
    jint composition_start, jint composition_end) {
    atomic_fetch_add(&text_reports, 1);
}
