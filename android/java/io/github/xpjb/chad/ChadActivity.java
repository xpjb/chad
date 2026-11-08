package io.github.xpjb.chad;

import android.app.NativeActivity;
import android.graphics.Color;
import android.os.Build;
import android.os.Bundle;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.KeyEvent;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsAnimation;
import android.view.WindowManager;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.CompletionInfo;
import android.view.inputmethod.CorrectionInfo;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputConnectionWrapper;
import android.view.inputmethod.InputMethodManager;
import android.view.inputmethod.TextAttribute;
import android.widget.EditText;
import android.widget.FrameLayout;

/** NativeActivity services shared by Chad applications: text input and animated visible bounds. */
public class ChadActivity extends NativeActivity {
    private static native void nativeTextInput(long id, long generation, long documentStart,
        String text, int selectionStart, int selectionEnd,
        int compositionStart, int compositionEnd);
    private static native void nativeViewport(int left, int top, int right, int bottom,
        boolean keyboardVisible);

    private MirrorInput input;
    private long inputId;
    private long inputGeneration;
    private long documentStart;
    private boolean inputMultiline;
    private boolean inputSecure;
    private boolean updatingInput;
    private boolean reportScheduled;
    private boolean reportPending;
    private int batchDepth;
    private final android.graphics.Rect lastViewport = new android.graphics.Rect();
    private boolean lastKeyboardVisible;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        try {
            android.content.pm.ActivityInfo info = getPackageManager().getActivityInfo(
                getComponentName(), android.content.pm.PackageManager.GET_META_DATA);
            String library = info.metaData == null ? null
                : info.metaData.getString("android.app.lib_name");
            if (library == null || library.isEmpty()) {
                throw new IllegalStateException("android.app.lib_name is not configured");
            }
            System.loadLibrary(library);
        } catch (android.content.pm.PackageManager.NameNotFoundException error) {
            throw new IllegalStateException("cannot read NativeActivity metadata", error);
        }
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        getWindow().setDecorFitsSystemWindows(false);
        View decor = getWindow().getDecorView();
        decor.setOnApplyWindowInsetsListener((view, insets) -> {
            reportViewport(insets);
            return insets;
        });
        decor.setWindowInsetsAnimationCallback(new WindowInsetsAnimation.Callback(
            WindowInsetsAnimation.Callback.DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
            @Override public WindowInsets onProgress(WindowInsets insets,
                java.util.List<WindowInsetsAnimation> runningAnimations) {
                reportViewport(insets);
                return insets;
            }
        });
        decor.getViewTreeObserver().addOnGlobalLayoutListener(this::reportViewport);
        decor.post(this::reportViewport);
    }

    private InputMethodManager keyboard() {
        return (InputMethodManager)getSystemService(INPUT_METHOD_SERVICE);
    }

    private void reportViewport() {
        reportViewport(getWindow().getDecorView().getRootWindowInsets());
    }

    private void reportViewport(WindowInsets insets) {
        if (insets == null) return;
        View decor = getWindow().getDecorView();
        android.graphics.Rect visible = new android.graphics.Rect(
            getWindowManager().getCurrentWindowMetrics().getBounds());
        android.graphics.Insets safe = insets.getInsets(
            WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout()
                | WindowInsets.Type.ime());
        visible.inset(safe.left, safe.top, safe.right, safe.bottom);
        int[] origin = new int[2];
        decor.getLocationOnScreen(origin);
        visible.offset(-origin[0], -origin[1]);
        boolean keyboardVisible = insets.isVisible(WindowInsets.Type.ime());
        if (visible.intersect(0, 0, decor.getWidth(), decor.getHeight())
            && (!lastViewport.equals(visible) || keyboardVisible != lastKeyboardVisible)) {
            lastViewport.set(visible);
            lastKeyboardVisible = keyboardVisible;
            nativeViewport(visible.left, visible.top, visible.right, visible.bottom,
                keyboardVisible);
        }
    }

    public void chadSetTextInput(long id, long generation, long start, String text,
        int anchor, int focus, float fieldX, float fieldY, float fieldWidth, float fieldHeight,
        float caretX, float caretY, float caretWidth, float caretHeight,
        boolean multiline, boolean secure, boolean reset) {
        runOnUiThread(() -> {
            if (input == null) createInput();
            boolean configuration = id != inputId || generation != inputGeneration
                || start != documentStart || multiline != inputMultiline || secure != inputSecure;
            boolean changed = reset || configuration
                || !input.getText().toString().equals(text);
            updatingInput = true;
            inputId = id;
            inputGeneration = generation;
            documentStart = start;
            inputMultiline = multiline;
            inputSecure = secure;
            if (configuration) {
                int type = InputType.TYPE_CLASS_TEXT;
                if (secure) {
                    type |= InputType.TYPE_TEXT_VARIATION_PASSWORD;
                } else if (multiline) {
                    type |= InputType.TYPE_TEXT_FLAG_MULTI_LINE
                        | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES;
                }
                input.setSingleLine(!multiline);
                input.setInputType(type);
                input.setImeOptions((multiline ? EditorInfo.IME_ACTION_NONE : EditorInfo.IME_ACTION_DONE)
                    | EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_FULLSCREEN
                    | (secure ? EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING : 0));
            }
            if (changed) input.setText(text);
            int length = input.length();
            int safeAnchor = Math.max(0, Math.min(length, anchor));
            int safeFocus = Math.max(0, Math.min(length, focus));
            if (input.getSelectionStart() != safeAnchor || input.getSelectionEnd() != safeFocus) {
                input.setSelection(safeAnchor, safeFocus);
            }

            View content = findViewById(android.R.id.content);
            int[] contentAt = new int[2];
            int[] decorAt = new int[2];
            content.getLocationInWindow(contentAt);
            getWindow().getDecorView().getLocationInWindow(decorAt);
            int width = Math.max(1, (int)Math.ceil(fieldWidth));
            int height = Math.max(1, (int)Math.ceil(fieldHeight));
            int left = Math.round(fieldX) + decorAt[0] - contentAt[0];
            int top = Math.round(fieldY) + decorAt[1] - contentAt[1];
            FrameLayout.LayoutParams layout = (FrameLayout.LayoutParams)input.getLayoutParams();
            if (layout.width != width || layout.height != height
                || layout.leftMargin != left || layout.topMargin != top) {
                layout.width = width;
                layout.height = height;
                layout.leftMargin = left;
                layout.topMargin = top;
                input.setLayoutParams(layout);
            }
            input.caretRect.set(
                (int)Math.floor(caretX - fieldX),
                (int)Math.floor(caretY - fieldY),
                (int)Math.ceil(caretX - fieldX + Math.max(1.0f, caretWidth)),
                (int)Math.ceil(caretY - fieldY + Math.max(1.0f, caretHeight)));
            if (input.getVisibility() != View.VISIBLE) input.setVisibility(View.VISIBLE);
            if (!input.hasFocus()) input.requestFocus();
            updatingInput = false;
            if (changed) keyboard().restartInput(input);
        });
    }

    public void chadClearTextInput() {
        runOnUiThread(() -> {
            inputId = 0;
            reportPending = false;
            if (input != null) {
                keyboard().hideSoftInputFromWindow(input.getWindowToken(), 0);
                input.clearFocus();
                input.setVisibility(View.GONE);
            }
        });
    }

    public void chadShowKeyboard() {
        runOnUiThread(() -> {
            if (inputId == 0 || input == null) return;
            input.setVisibility(View.VISIBLE);
            input.requestFocus();
            input.post(() -> {
                if (inputId != 0 && input.hasFocus()) {
                    keyboard().showSoftInput(input, InputMethodManager.SHOW_IMPLICIT);
                }
            });
        });
    }

    public void chadHideKeyboard() {
        runOnUiThread(() -> {
            if (input != null) keyboard().hideSoftInputFromWindow(input.getWindowToken(), 0);
        });
    }

    private void createInput() {
        input = new MirrorInput();
        input.setAlpha(0.0f);
        input.setTextColor(Color.TRANSPARENT);
        input.setHighlightColor(Color.TRANSPARENT);
        input.setCursorVisible(false);
        input.setBackground(null);
        input.setShowSoftInputOnFocus(false);
        input.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS);
        input.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence value, int start, int count,
                int after) {}
            @Override public void onTextChanged(CharSequence value, int start, int before,
                int count) {}
            @Override public void afterTextChanged(Editable value) { scheduleReport(); }
        });
        addContentView(input, new FrameLayout.LayoutParams(1, 1));
    }

    private void scheduleReport() {
        if (updatingInput || inputId == 0 || input == null) return;
        if (batchDepth > 0) {
            reportPending = true;
            return;
        }
        if (reportScheduled) return;
        reportScheduled = true;
        input.post(() -> {
            reportScheduled = false;
            reportEdit();
        });
    }

    private void reportEdit() {
        if (updatingInput || inputId == 0 || input == null) return;
        Editable value = input.getText();
        nativeTextInput(inputId, inputGeneration, documentStart, value.toString(),
            input.getSelectionStart(), input.getSelectionEnd(),
            BaseInputConnection.getComposingSpanStart(value),
            BaseInputConnection.getComposingSpanEnd(value));
    }

    private final class MirrorInput extends EditText {
        final android.graphics.Rect caretRect = new android.graphics.Rect();

        MirrorInput() { super(ChadActivity.this); }

        @Override protected void onSelectionChanged(int start, int end) {
            super.onSelectionChanged(start, end);
            scheduleReport();
        }

        @Override public boolean requestRectangleOnScreen(android.graphics.Rect rectangle,
            boolean immediate) {
            return false;
        }

        @Override public boolean onTouchEvent(android.view.MotionEvent event) {
            return false;
        }

        @Override public InputConnection onCreateInputConnection(EditorInfo info) {
            InputConnection connection = super.onCreateInputConnection(info);
            if (connection == null) return null;
            if (Build.VERSION.SDK_INT >= 34) return new ReplacementConnection(connection);
            if (Build.VERSION.SDK_INT >= 33) return new AttributedConnection(connection);
            return new MirrorConnection(connection);
        }
    }

    private class MirrorConnection extends InputConnectionWrapper {
        private final long id = inputId;
        private final long generation = inputGeneration;
        private final long start = documentStart;

        MirrorConnection(InputConnection connection) { super(connection, false); }

        boolean active() {
            return inputId == id && inputGeneration == generation && documentStart == start
                && id != 0;
        }

        boolean report(boolean result) {
            scheduleReport();
            return result;
        }

        @Override public boolean beginBatchEdit() {
            if (!active() || !super.beginBatchEdit()) return false;
            batchDepth += 1;
            return true;
        }

        @Override public boolean endBatchEdit() {
            if (!active()) return false;
            boolean result = super.endBatchEdit();
            batchDepth = Math.max(0, batchDepth - 1);
            if (batchDepth == 0 && reportPending) {
                reportPending = false;
                scheduleReport();
            }
            return result;
        }

        @Override public boolean setComposingText(CharSequence text, int position) {
            return active() && report(super.setComposingText(text, position));
        }
        @Override public boolean setComposingRegion(int start, int end) {
            return active() && report(super.setComposingRegion(start, end));
        }
        @Override public boolean finishComposingText() {
            return active() && report(super.finishComposingText());
        }
        @Override public boolean commitText(CharSequence text, int position) {
            return active() && report(super.commitText(text, position));
        }
        @Override public boolean setSelection(int start, int end) {
            return active() && report(super.setSelection(start, end));
        }
        @Override public boolean deleteSurroundingText(int before, int after) {
            return active() && report(super.deleteSurroundingText(before, after));
        }
        @Override public boolean deleteSurroundingTextInCodePoints(int before, int after) {
            return active() && report(super.deleteSurroundingTextInCodePoints(before, after));
        }
        @Override public boolean sendKeyEvent(KeyEvent event) {
            return active() && report(super.sendKeyEvent(event));
        }
        @Override public boolean performContextMenuAction(int action) {
            return active() && report(super.performContextMenuAction(action));
        }
        @Override public boolean performEditorAction(int action) {
            return active() && report(super.performEditorAction(action));
        }
        @Override public boolean commitCompletion(CompletionInfo completion) {
            return active() && report(super.commitCompletion(completion));
        }
        @Override public boolean commitCorrection(CorrectionInfo correction) {
            return active() && report(super.commitCorrection(correction));
        }
        @Override public void closeConnection() {
            if (active()) super.closeConnection();
        }
    }

    private class AttributedConnection extends MirrorConnection {
        AttributedConnection(InputConnection connection) { super(connection); }

        @Override public boolean commitText(CharSequence text, int position,
            TextAttribute attributes) {
            return active() && report(super.commitText(text, position, attributes));
        }
        @Override public boolean setComposingText(CharSequence text, int position,
            TextAttribute attributes) {
            return active() && report(super.setComposingText(text, position, attributes));
        }
        @Override public boolean setComposingRegion(int start, int end,
            TextAttribute attributes) {
            return active() && report(super.setComposingRegion(start, end, attributes));
        }
    }

    private final class ReplacementConnection extends AttributedConnection {
        ReplacementConnection(InputConnection connection) { super(connection); }

        @Override public boolean replaceText(int start, int end, CharSequence text, int position,
            TextAttribute attributes) {
            return active() && report(super.replaceText(start, end, text, position, attributes));
        }
    }
}
