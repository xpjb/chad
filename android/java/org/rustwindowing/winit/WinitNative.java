package org.rustwindowing.winit;

import android.app.Activity;
import android.view.Surface;

final class WinitNative {
    static final int POINTER_DOWN = 0;
    static final int POINTER_MOVE = 1;
    static final int POINTER_UP = 2;
    static final int POINTER_CANCEL = 3;

    private WinitNative() {}

    static native long nativeCreate(Activity activity, String filesDir);
    static native void nativeDestroy(long handle);
    static native void nativeStart(long handle);
    static native void nativeStop(long handle);
    static native void nativeFrame(long handle);
    static native void nativeSurfaceCreated(
            long handle, Surface surface, int width, int height, double scaleFactor);
    static native void nativeSurfaceChanged(
            long handle, int width, int height, double scaleFactor);
    static native void nativeSurfaceDestroyed(long handle);
    static native void nativeFocus(long handle, boolean focused);
    static native void nativeInsets(long handle, int left, int top, int right, int bottom);
    static native boolean nativePointer(
            long handle,
            int action,
            long timeNanos,
            int deviceId,
            int pointerId,
            float x,
            float y,
            float pressure);
    static native boolean nativeKey(
            long handle,
            boolean pressed,
            int deviceId,
            int keyCode,
            long unicode,
            long unicodeWithoutModifiers,
            boolean repeat);
    static native boolean nativeBack(long handle);
    static native boolean nativeTakeWork(long handle);
    static native boolean nativeExiting(long handle);
    static native long[] nativeTakeImeCommand(long handle);
    static native long[] nativeTextState(long handle, long sessionId);
    static native String nativeTextRead(long handle, long sessionId, long start, long end);
    static native long[] nativeTextApply(
            long handle,
            long sessionId,
            boolean hasReplacement,
            long replacementStart,
            long replacementEnd,
            String replacementText,
            long selectionAnchor,
            long selectionFocus,
            long compositionStart,
            long compositionEnd);
}
