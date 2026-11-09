package io.github.xpjb.chad;

import android.app.NativeActivity;
import android.graphics.Color;
import android.os.Bundle;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.ActionMode;
import android.view.Menu;
import android.view.MenuItem;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowInsetsAnimation;
import android.view.WindowManager;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
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
    private ActionMode inputMenu;
    private boolean updatingInput;
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
        boolean multiline, boolean secure) {
        runOnUiThread(() -> {
            boolean configuration = input == null || input.id != id
                || input.generation != generation || input.documentStart != start
                || input.multiline != multiline || input.secure != secure;
            if (configuration && inputMenu != null) {
                inputMenu.finish();
                inputMenu = null;
            }
            updatingInput = true;
            if (configuration) {
                MirrorInput previous = input;
                input = new MirrorInput(id, generation, start, multiline, secure);
                addContentView(input, new FrameLayout.LayoutParams(1, 1));
                if (previous != null) {
                    previous.clearFocus();
                    if (previous.getParent() instanceof ViewGroup) {
                        ((ViewGroup)previous.getParent()).removeView(previous);
                    }
                }
            }
            boolean changed = configuration || !input.getText().toString().equals(text);
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
            if (inputMenu != null) {
                inputMenu.invalidate();
                inputMenu.invalidateContentRect();
            }
            if (!input.hasFocus()) input.requestFocus();
            updatingInput = false;
            if (changed) keyboard().restartInput(input);
        });
    }

    public void chadClearTextInput() {
        runOnUiThread(() -> {
            if (inputMenu != null) {
                inputMenu.finish();
                inputMenu = null;
            }
            MirrorInput previous = input;
            input = null;
            if (previous != null) {
                keyboard().hideSoftInputFromWindow(previous.getWindowToken(), 0);
                previous.clearFocus();
                if (previous.getParent() instanceof ViewGroup) {
                    ((ViewGroup)previous.getParent()).removeView(previous);
                }
            }
        });
    }

    public void chadShowKeyboard() {
        runOnUiThread(() -> {
            MirrorInput target = input;
            if (target == null) return;
            target.requestFocus();
            target.post(() -> {
                if (input == target && target.hasFocus()) {
                    keyboard().showSoftInput(target, InputMethodManager.SHOW_IMPLICIT);
                }
            });
        });
    }

    public void chadHideKeyboard() {
        runOnUiThread(() -> {
            if (input != null) keyboard().hideSoftInputFromWindow(input.getWindowToken(), 0);
        });
    }

    public boolean chadHideKeyboardIfVisible() {
        if (!lastKeyboardVisible) return false;
        chadHideKeyboard();
        return true;
    }

    public void chadMoveTaskToBack() {
        runOnUiThread(() -> moveTaskToBack(true));
    }

    public void chadShowTextMenu() {
        runOnUiThread(() -> {
            final MirrorInput target = input;
            if (target == null) return;
            if (inputMenu != null) inputMenu.finish();
            inputMenu = target.startActionMode(new ActionMode.Callback2() {
                private boolean active() {
                    return input == target;
                }

                @Override public boolean onCreateActionMode(ActionMode mode, Menu menu) {
                    menu.add(0, android.R.id.cut, 0, android.R.string.cut)
                        .setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM);
                    menu.add(0, android.R.id.copy, 1, android.R.string.copy)
                        .setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM);
                    menu.add(0, android.R.id.paste, 2, android.R.string.paste)
                        .setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM);
                    menu.add(0, android.R.id.selectAll, 3, android.R.string.selectAll)
                        .setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM);
                    return active();
                }

                @Override public boolean onPrepareActionMode(ActionMode mode, Menu menu) {
                    boolean selected = target.getSelectionStart() != target.getSelectionEnd();
                    menu.findItem(android.R.id.cut).setVisible(selected && !target.secure);
                    menu.findItem(android.R.id.copy).setVisible(selected && !target.secure);
                    menu.findItem(android.R.id.paste).setEnabled(
                        ((android.content.ClipboardManager)getSystemService(CLIPBOARD_SERVICE))
                            .hasPrimaryClip());
                    menu.findItem(android.R.id.selectAll).setEnabled(target.length() > 0);
                    return active();
                }

                @Override public boolean onActionItemClicked(ActionMode mode, MenuItem item) {
                    if (!active()) {
                        mode.finish();
                        return false;
                    }
                    if (item.getItemId() == android.R.id.selectAll) {
                        target.setSelection(0, target.length());
                        mode.invalidate();
                    } else {
                        target.onTextContextMenuItem(item.getItemId());
                        mode.finish();
                    }
                    return true;
                }

                @Override public void onGetContentRect(ActionMode mode, View view,
                    android.graphics.Rect rectangle) {
                    rectangle.set(target.caretRect);
                }

                @Override public void onDestroyActionMode(ActionMode mode) {
                    if (inputMenu == mode) inputMenu = null;
                }
            }, ActionMode.TYPE_FLOATING);
        });
    }

    private void scheduleReport(MirrorInput source) {
        if (updatingInput || source != input || source.reportScheduled) return;
        source.reportScheduled = true;
        source.post(() -> {
            source.reportScheduled = false;
            if (updatingInput || source != input) return;
            Editable value = source.getText();
            nativeTextInput(source.id, source.generation, source.documentStart, value.toString(),
                source.getSelectionStart(), source.getSelectionEnd(),
                BaseInputConnection.getComposingSpanStart(value),
                BaseInputConnection.getComposingSpanEnd(value));
        });
    }

    private final class MirrorInput extends EditText {
        final long id;
        final long generation;
        final long documentStart;
        final boolean multiline;
        final boolean secure;
        final android.graphics.Rect caretRect = new android.graphics.Rect();
        boolean reportScheduled;

        MirrorInput(long id, long generation, long documentStart,
            boolean multiline, boolean secure) {
            super(ChadActivity.this);
            this.id = id;
            this.generation = generation;
            this.documentStart = documentStart;
            this.multiline = multiline;
            this.secure = secure;
            int type = InputType.TYPE_CLASS_TEXT;
            if (secure) {
                type |= InputType.TYPE_TEXT_VARIATION_PASSWORD;
            } else if (multiline) {
                type |= InputType.TYPE_TEXT_FLAG_MULTI_LINE
                    | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES;
            }
            setSingleLine(!multiline);
            setInputType(type);
            setImeOptions((multiline ? EditorInfo.IME_ACTION_NONE : EditorInfo.IME_ACTION_DONE)
                | EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_FULLSCREEN
                | (secure ? EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING : 0));
            setAlpha(0.0f);
            setTextColor(Color.TRANSPARENT);
            setHighlightColor(Color.TRANSPARENT);
            setCursorVisible(false);
            setBackground(null);
            setShowSoftInputOnFocus(false);
            setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS);
            addTextChangedListener(new TextWatcher() {
                @Override public void beforeTextChanged(CharSequence value, int start, int count,
                    int after) {}
                @Override public void onTextChanged(CharSequence value, int start, int before,
                    int count) {}
                @Override public void afterTextChanged(Editable value) {
                    scheduleReport(MirrorInput.this);
                }
            });
        }

        @Override protected void onSelectionChanged(int start, int end) {
            super.onSelectionChanged(start, end);
            scheduleReport(this);
        }

        @Override public boolean requestRectangleOnScreen(android.graphics.Rect rectangle,
            boolean immediate) {
            return false;
        }

        @Override public boolean onTouchEvent(android.view.MotionEvent event) {
            return false;
        }
    }
}
