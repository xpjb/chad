package org.rustwindowing.winit;

import android.text.TextUtils;
import android.view.inputmethod.SurroundingText;
import android.view.inputmethod.TextAttribute;
import android.view.inputmethod.TextSnapshot;

class WinitInputConnectionApi33 extends WinitInputConnectionApi31 {
    WinitInputConnectionApi33(
            WinitActivity activity,
            WinitActivity.WinitSurfaceView view,
            long nativeHandle,
            long sessionId,
            boolean multiline,
            boolean secure) {
        super(activity, view, nativeHandle, sessionId, multiline, secure);
    }

    @Override
    public boolean setComposingText(
            CharSequence text, int newCursorPosition, TextAttribute textAttribute) {
        return setComposingText(text, newCursorPosition);
    }

    @Override
    public boolean setComposingRegion(int start, int end, TextAttribute textAttribute) {
        return setComposingRegion(start, end);
    }

    @Override
    public boolean commitText(
            CharSequence text, int newCursorPosition, TextAttribute textAttribute) {
        return commitText(text, newCursorPosition);
    }

    @Override
    public TextSnapshot takeSnapshot() {
        SurroundingSlice slice = surroundingText(2048, 2048);
        if (slice == null) {
            return null;
        }
        SurroundingText surrounding =
                new SurroundingText(
                        slice.text,
                        Math.min(slice.selectionAnchor, slice.selectionFocus),
                        Math.max(slice.selectionAnchor, slice.selectionFocus),
                        slice.offset);
        return new TextSnapshot(
                surrounding,
                slice.compositionStart,
                slice.compositionEnd,
                getCursorCapsMode(
                        TextUtils.CAP_MODE_CHARACTERS
                                | TextUtils.CAP_MODE_WORDS
                                | TextUtils.CAP_MODE_SENTENCES));
    }
}
