package org.rustwindowing.winit;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.text.Editable;
import android.text.TextUtils;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.CompletionInfo;
import android.view.inputmethod.CorrectionInfo;
import android.view.inputmethod.ExtractedText;
import android.view.inputmethod.ExtractedTextRequest;

/** InputConnection whose source of truth is the application text client in Rust. */
class WinitInputConnection extends BaseInputConnection {
    private final WinitActivity activity;
    private final long nativeHandle;
    private final long sessionId;
    private final boolean multiline;
    private final boolean secure;
    private int batchDepth;
    private boolean closed;
    private boolean selectionUpdatePending;
    private TextState cachedState;

    WinitInputConnection(
            WinitActivity activity,
            View view,
            long nativeHandle,
            long sessionId,
            boolean multiline,
            boolean secure) {
        super(view, true);
        this.activity = activity;
        this.nativeHandle = nativeHandle;
        this.sessionId = sessionId;
        this.multiline = multiline;
        this.secure = secure;
    }

    @Override
    public Editable getEditable() {
        // BaseInputConnection must not create a private text mirror. Every supported operation below
        // talks directly to the Rust text client.
        return null;
    }

    @Override
    public CharSequence getTextBeforeCursor(int length, int flags) {
        TextState state = state();
        if (state == null) {
            return "";
        }
        int cursor = state.selectionStart();
        return read(Math.max(0, cursor - Math.max(0, length)), cursor);
    }

    @Override
    public CharSequence getTextAfterCursor(int length, int flags) {
        TextState state = state();
        if (state == null) {
            return "";
        }
        int cursor = state.selectionEnd();
        return read(cursor, Math.min(state.textLength, cursor + Math.max(0, length)));
    }

    @Override
    public CharSequence getSelectedText(int flags) {
        TextState state = state();
        if (state == null || state.selectionStart() == state.selectionEnd()) {
            return null;
        }
        return read(state.selectionStart(), state.selectionEnd());
    }

    @Override
    public ExtractedText getExtractedText(ExtractedTextRequest request, int flags) {
        TextState state = state();
        if (state == null) {
            return null;
        }
        ExtractedText extracted = new ExtractedText();
        extracted.text = read(0, state.textLength);
        extracted.startOffset = 0;
        extracted.partialStartOffset = -1;
        extracted.partialEndOffset = -1;
        extracted.selectionStart = state.anchor;
        extracted.selectionEnd = state.focus;
        extracted.flags = multiline ? 0 : ExtractedText.FLAG_SINGLE_LINE;
        return extracted;
    }

    @Override
    public boolean commitText(CharSequence text, int newCursorPosition) {
        TextState state = state();
        if (state == null) {
            return false;
        }
        String replacement = text == null ? "" : text.toString();
        int start = state.replacementStart();
        int end = state.replacementEnd();
        int newLength = state.textLength - (end - start) + replacement.length();
        int cursor = cursorAfterReplacement(start, replacement.length(), newCursorPosition, newLength);
        return apply(start, end, replacement, cursor, cursor, -1, -1);
    }

    @Override
    public boolean setComposingText(CharSequence text, int newCursorPosition) {
        TextState state = state();
        if (state == null) {
            return false;
        }
        String replacement = text == null ? "" : text.toString();
        int start = state.replacementStart();
        int end = state.replacementEnd();
        int newLength = state.textLength - (end - start) + replacement.length();
        int cursor = cursorAfterReplacement(start, replacement.length(), newCursorPosition, newLength);
        int compositionEnd = start + replacement.length();
        return apply(
                start,
                end,
                replacement,
                cursor,
                cursor,
                replacement.isEmpty() ? -1 : start,
                replacement.isEmpty() ? -1 : compositionEnd);
    }

    @Override
    public boolean setComposingRegion(int start, int end) {
        TextState state = state();
        if (state == null) {
            return false;
        }
        int orderedStart = Math.max(0, Math.min(start, end));
        int orderedEnd = Math.min(state.textLength, Math.max(start, end));
        if (orderedStart > orderedEnd) {
            orderedStart = orderedEnd;
        }
        return apply(
                -1,
                -1,
                null,
                state.anchor,
                state.focus,
                orderedStart,
                orderedEnd);
    }

    @Override
    public boolean finishComposingText() {
        TextState state = state();
        return state != null
                && apply(-1, -1, null, state.anchor, state.focus, -1, -1);
    }

    @Override
    public boolean setSelection(int start, int end) {
        TextState state = state();
        if (state == null) {
            return false;
        }
        int anchor = clamp(start, 0, state.textLength);
        int focus = clamp(end, 0, state.textLength);
        return apply(
                -1,
                -1,
                null,
                anchor,
                focus,
                state.compositionStart,
                state.compositionEnd);
    }

    @Override
    public boolean deleteSurroundingText(int beforeLength, int afterLength) {
        TextState state = state();
        if (state == null) {
            return false;
        }
        int start = Math.max(0, state.selectionStart() - Math.max(0, beforeLength));
        int end = Math.min(state.textLength, state.selectionEnd() + Math.max(0, afterLength));
        return apply(start, end, "", start, start, -1, -1);
    }

    @Override
    public boolean deleteSurroundingTextInCodePoints(int beforeLength, int afterLength) {
        TextState state = state();
        if (state == null) {
            return false;
        }

        int selectionStart = state.selectionStart();
        int selectionEnd = state.selectionEnd();
        int beforeProbeStart = Math.max(0, selectionStart - Math.max(0, beforeLength) * 2);
        String before = read(beforeProbeStart, selectionStart);
        int beforeUnits =
                before.length()
                        - before.offsetByCodePoints(
                                before.length(), -Math.min(before.codePointCount(0, before.length()), beforeLength));

        int afterProbeEnd = Math.min(state.textLength, selectionEnd + Math.max(0, afterLength) * 2);
        String after = read(selectionEnd, afterProbeEnd);
        int afterCodePoints = Math.min(after.codePointCount(0, after.length()), afterLength);
        int afterUnits = after.offsetByCodePoints(0, afterCodePoints);

        int start = selectionStart - beforeUnits;
        int end = selectionEnd + afterUnits;
        return apply(start, end, "", start, start, -1, -1);
    }

    @Override
    public int getCursorCapsMode(int reqModes) {
        TextState state = state();
        if (state == null) {
            return 0;
        }
        int cursor = state.selectionStart();
        int start = Math.max(0, cursor - 2048);
        String before = read(start, cursor);
        return TextUtils.getCapsMode(before, before.length(), reqModes);
    }

    @Override
    public boolean beginBatchEdit() {
        if (closed) {
            return false;
        }
        batchDepth++;
        return true;
    }

    @Override
    public boolean endBatchEdit() {
        if (batchDepth > 0) {
            batchDepth--;
        }
        if (batchDepth == 0 && selectionUpdatePending) {
            publishSelection(cachedState);
        }
        return !closed;
    }

    @Override
    public boolean commitCompletion(CompletionInfo completion) {
        return completion != null && commitText(completion.getText(), 1);
    }

    @Override
    public boolean commitCorrection(CorrectionInfo correctionInfo) {
        if (correctionInfo == null) {
            return false;
        }
        CharSequence oldText = correctionInfo.getOldText();
        return replaceTextRange(
                correctionInfo.getOffset(),
                correctionInfo.getOffset() + (oldText == null ? 0 : oldText.length()),
                correctionInfo.getNewText(),
                1);
    }

    @Override
    public boolean performContextMenuAction(int action) {
        TextState state = state();
        if (state == null) {
            return false;
        }
        int start = state.selectionStart();
        int end = state.selectionEnd();
        if (action == android.R.id.selectAll) {
            return apply(-1, -1, null, 0, state.textLength, -1, -1);
        }
        if (action == android.R.id.copy || action == android.R.id.cut) {
            if (secure || start == end) {
                return false;
            }
            ClipboardManager clipboard = activity.getSystemService(ClipboardManager.class);
            clipboard.setPrimaryClip(ClipData.newPlainText(null, read(start, end)));
            return action != android.R.id.cut
                    || apply(start, end, "", start, start, -1, -1);
        }
        if (action == android.R.id.paste || action == android.R.id.pasteAsPlainText) {
            ClipboardManager clipboard = activity.getSystemService(ClipboardManager.class);
            if (!clipboard.hasPrimaryClip() || clipboard.getPrimaryClip() == null
                    || clipboard.getPrimaryClip().getItemCount() == 0) {
                return false;
            }
            CharSequence value = clipboard.getPrimaryClip().getItemAt(0).coerceToText(activity);
            return commitText(value == null ? "" : value, 1);
        }
        return false;
    }

    @Override
    public boolean clearMetaKeyStates(int states) {
        return !closed;
    }

    @Override
    public boolean sendKeyEvent(KeyEvent event) {
        // Input methods may represent Backspace, Delete, Enter, and navigation as key events
        // instead of direct InputConnection edits. Dispatch through the Activity so the event
        // reaches Winit's native key bridge; dispatching only to the bare SurfaceView bypasses it.
        return !closed && activity.dispatchKeyEvent(event);
    }

    @Override
    public void closeConnection() {
        closed = true;
        selectionUpdatePending = false;
        cachedState = null;
        activity.textConnectionClosed(this);
        super.closeConnection();
    }

    final long sessionId() {
        return sessionId;
    }

    final boolean isSecure() {
        return secure;
    }

    final boolean hasSelection() {
        TextState state = state();
        return state != null && state.selectionStart() != state.selectionEnd();
    }

    final boolean hasText() {
        TextState state = state();
        return state != null && state.textLength > 0;
    }

    final TextState state() {
        if (closed || !activity.isTextSessionActive(sessionId)) {
            return null;
        }
        long[] wire = WinitNative.nativeTextState(nativeHandle, sessionId);
        activity.afterTextInputCall();
        TextState state = TextState.fromWire(wire);
        if (state == null) {
            closed = true;
        } else {
            cachedState = state;
        }
        return state;
    }

    final String read(int start, int end) {
        if (closed || end < start) {
            return "";
        }
        String text = WinitNative.nativeTextRead(nativeHandle, sessionId, start, end);
        activity.afterTextInputCall();
        return text == null ? "" : text;
    }

    final SurroundingSlice surroundingText(int beforeLength, int afterLength) {
        TextState state = state();
        if (state == null) {
            return null;
        }
        int selectionStart = state.selectionStart();
        int selectionEnd = state.selectionEnd();
        int start = Math.max(0, selectionStart - Math.min(Math.max(0, beforeLength), 2048));
        int end =
                Math.min(
                        state.textLength,
                        selectionEnd + Math.min(Math.max(0, afterLength), 2048));
        boolean compositionIncluded =
                state.compositionStart >= start && state.compositionEnd <= end;
        return new SurroundingSlice(
                read(start, end),
                start,
                state.anchor - start,
                state.focus - start,
                compositionIncluded ? state.compositionStart - start : -1,
                compositionIncluded ? state.compositionEnd - start : -1);
    }

    final boolean replaceTextRange(
            int requestedStart,
            int requestedEnd,
            CharSequence text,
            int newCursorPosition) {
        TextState state = state();
        if (state == null) {
            return false;
        }
        int start = clamp(Math.min(requestedStart, requestedEnd), 0, state.textLength);
        int end = clamp(Math.max(requestedStart, requestedEnd), 0, state.textLength);
        String replacement = text == null ? "" : text.toString();
        int newLength = state.textLength - (end - start) + replacement.length();
        int cursor = cursorAfterReplacement(start, replacement.length(), newCursorPosition, newLength);
        return apply(start, end, replacement, cursor, cursor, -1, -1);
    }

    final boolean apply(
            int replacementStart,
            int replacementEnd,
            String replacement,
            int selectionAnchor,
            int selectionFocus,
            int compositionStart,
            int compositionEnd) {
        if (closed || !activity.isTextSessionActive(sessionId)) {
            return false;
        }
        boolean hasReplacement = replacement != null;
        long[] wire =
                WinitNative.nativeTextApply(
                        nativeHandle,
                        sessionId,
                        hasReplacement,
                        hasReplacement ? replacementStart : 0,
                        hasReplacement ? replacementEnd : 0,
                        hasReplacement ? replacement : "",
                        selectionAnchor,
                        selectionFocus,
                        compositionStart,
                        compositionEnd);
        activity.afterTextInputCall();
        TextState state = TextState.fromWire(wire);
        if (state == null) {
            closed = true;
            return false;
        }
        cachedState = state;
        selectionUpdatePending = true;
        if (batchDepth == 0) {
            publishSelection(state);
        }
        return wire[0] == 1;
    }

    private void publishSelection(TextState state) {
        if (state == null || closed) {
            return;
        }
        selectionUpdatePending = false;
        activity.updateTextSelection(
                sessionId,
                state.anchor,
                state.focus,
                state.compositionStart,
                state.compositionEnd);
    }

    private static int cursorAfterReplacement(
            int replacementStart, int insertedLength, int requested, int documentLength) {
        int cursor;
        if (requested > 0) {
            cursor = replacementStart + insertedLength + requested - 1;
        } else {
            cursor = replacementStart + requested;
        }
        return clamp(cursor, 0, documentLength);
    }

    private static int clamp(int value, int minimum, int maximum) {
        return Math.max(minimum, Math.min(maximum, value));
    }

    static final class SurroundingSlice {
        final String text;
        final int offset;
        final int selectionAnchor;
        final int selectionFocus;
        final int compositionStart;
        final int compositionEnd;

        SurroundingSlice(
                String text,
                int offset,
                int selectionAnchor,
                int selectionFocus,
                int compositionStart,
                int compositionEnd) {
            this.text = text;
            this.offset = offset;
            this.selectionAnchor = selectionAnchor;
            this.selectionFocus = selectionFocus;
            this.compositionStart = compositionStart;
            this.compositionEnd = compositionEnd;
        }
    }

    static final class TextState {
        final int textLength;
        final int anchor;
        final int focus;
        final int compositionStart;
        final int compositionEnd;

        TextState(
                int textLength,
                int anchor,
                int focus,
                int compositionStart,
                int compositionEnd) {
            this.textLength = textLength;
            this.anchor = anchor;
            this.focus = focus;
            this.compositionStart = compositionStart;
            this.compositionEnd = compositionEnd;
        }

        int selectionStart() {
            return Math.min(anchor, focus);
        }

        int selectionEnd() {
            return Math.max(anchor, focus);
        }

        int replacementStart() {
            return compositionStart >= 0 ? compositionStart : selectionStart();
        }

        int replacementEnd() {
            return compositionEnd >= 0 ? compositionEnd : selectionEnd();
        }

        static TextState fromWire(long[] wire) {
            if (wire == null || wire.length < 6 || wire[0] < 1 || wire[0] > 2) {
                return null;
            }
            return new TextState(
                    safeInt(wire[1]),
                    safeInt(wire[2]),
                    safeInt(wire[3]),
                    safeInt(wire[4]),
                    safeInt(wire[5]));
        }

        private static int safeInt(long value) {
            if (value < Integer.MIN_VALUE || value > Integer.MAX_VALUE) {
                throw new IllegalStateException("text index exceeds Android's integer range");
            }
            return (int) value;
        }
    }
}
