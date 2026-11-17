package org.rustwindowing.winit;

import android.view.inputmethod.TextAttribute;

class WinitInputConnectionApi34 extends WinitInputConnectionApi33 {
    WinitInputConnectionApi34(
            WinitActivity activity,
            WinitActivity.WinitSurfaceView view,
            long nativeHandle,
            long sessionId,
            boolean multiline,
            boolean secure) {
        super(activity, view, nativeHandle, sessionId, multiline, secure);
    }

    @Override
    public boolean replaceText(
            int start,
            int end,
            CharSequence text,
            int newCursorPosition,
            TextAttribute textAttribute) {
        return replaceTextRange(start, end, text, newCursorPosition);
    }
}
