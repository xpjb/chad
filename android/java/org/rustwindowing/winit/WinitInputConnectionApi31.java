package org.rustwindowing.winit;

import android.view.inputmethod.SurroundingText;

class WinitInputConnectionApi31 extends WinitInputConnection {
    WinitInputConnectionApi31(
            WinitActivity activity,
            WinitActivity.WinitSurfaceView view,
            long nativeHandle,
            long sessionId,
            boolean multiline,
            boolean secure) {
        super(activity, view, nativeHandle, sessionId, multiline, secure);
    }

    @Override
    public SurroundingText getSurroundingText(int beforeLength, int afterLength, int flags) {
        SurroundingSlice slice = surroundingText(beforeLength, afterLength);
        if (slice == null) {
            return null;
        }
        return new SurroundingText(
                slice.text,
                Math.min(slice.selectionAnchor, slice.selectionFocus),
                Math.max(slice.selectionAnchor, slice.selectionFocus),
                slice.offset);
    }
}
