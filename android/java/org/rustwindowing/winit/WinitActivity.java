package org.rustwindowing.winit;

import android.app.Activity;
import android.content.ClipboardManager;
import android.graphics.Insets;
import android.graphics.Rect;
import android.os.Build;
import android.os.Bundle;
import android.text.InputType;
import android.view.ActionMode;
import android.view.Choreographer;
import android.view.HapticFeedbackConstants;
import android.view.KeyEvent;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsAnimation;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import java.util.List;

/**
 * Experimental Winit Activity whose Java UI Looper owns application dispatch.
 *
 * <p>Subclasses may override {@link #nativeLibraryName()} when their Rust cdylib is not named
 * {@code main}.</p>
 */
public class WinitActivity extends Activity
        implements SurfaceHolder.Callback, Choreographer.FrameCallback {
    private WinitSurfaceView surfaceView;
    private long nativeHandle;
    private boolean framePosted;
    private boolean wakePosted;
    private boolean textDrainPosted;
    private long activeTextSession;
    private int textHint;
    private int textPurpose;
    private ActionMode inputMenu;
    private long inputMenuSession = -1;

    protected String nativeLibraryName() {
        try {
            android.content.pm.ActivityInfo info =
                    getPackageManager()
                            .getActivityInfo(
                                    getComponentName(),
                                    android.content.pm.PackageManager.GET_META_DATA);
            String configured =
                    info.metaData == null
                            ? null
                            : info.metaData.getString("android.app.lib_name");
            if (configured != null && !configured.isEmpty()) {
                return configured;
            }
        } catch (android.content.pm.PackageManager.NameNotFoundException error) {
            throw new IllegalStateException("cannot read Activity metadata", error);
        }
        return "main";
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        System.loadLibrary(nativeLibraryName());

        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
        }
        surfaceView = new WinitSurfaceView(this);
        surfaceView.setFocusable(true);
        surfaceView.setFocusableInTouchMode(true);
        setContentView(surfaceView);

        nativeHandle = WinitNative.nativeCreate(this, getFilesDir().getAbsolutePath());
        surfaceView.setNativeHandle(nativeHandle);
        surfaceView.getHolder().addCallback(this);
        installInsetsListener();
        afterNativeCall();
    }

    @Override
    protected void onStart() {
        super.onStart();
        if (nativeHandle != 0) {
            WinitNative.nativeStart(nativeHandle);
            afterNativeCall();
        }
    }

    @Override
    protected void onStop() {
        if (nativeHandle != 0) {
            WinitNative.nativeStop(nativeHandle);
            afterNativeCall();
        }
        super.onStop();
    }

    @Override
    protected void onDestroy() {
        finishTextMenu();
        if (nativeHandle != 0) {
            surfaceView.getHolder().removeCallback(this);
            Choreographer.getInstance().removeFrameCallback(this);
            WinitNative.nativeDestroy(nativeHandle);
            nativeHandle = 0;
        }
        super.onDestroy();
    }

    @Override
    public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        if (nativeHandle != 0) {
            WinitNative.nativeFocus(nativeHandle, focused);
            afterNativeCall();
        }
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        if (nativeHandle == 0) {
            return;
        }
        WinitNative.nativeSurfaceCreated(
                nativeHandle,
                holder.getSurface(),
                surfaceView.getWidth(),
                surfaceView.getHeight(),
                getResources().getDisplayMetrics().density);
        surfaceView.requestApplyInsets();
        afterNativeCall();
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        if (nativeHandle == 0) {
            return;
        }
        WinitNative.nativeSurfaceChanged(
                nativeHandle, width, height, getResources().getDisplayMetrics().density);
        afterNativeCall();
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        if (nativeHandle == 0) {
            return;
        }
        WinitNative.nativeSurfaceDestroyed(nativeHandle);
        afterNativeCall();
    }

    @Override
    public void doFrame(long frameTimeNanos) {
        framePosted = false;
        if (nativeHandle == 0) {
            return;
        }
        WinitNative.nativeFrame(nativeHandle);
        afterNativeCall();
    }

    /** Called by Rust only to enqueue a fresh Java stack; it never calls Rust inline. */
    public final void requestRustWake() {
        if (surfaceView == null || wakePosted) {
            return;
        }
        wakePosted = true;
        surfaceView.post(
                () -> {
                    wakePosted = false;
                    afterNativeCall();
                });
    }

    void afterPointerCall() {
        afterNativeCall();
    }

    void afterTextInputCall() {
        if (surfaceView == null || textDrainPosted) {
            return;
        }
        textDrainPosted = true;
        surfaceView.post(
                () -> {
                    textDrainPosted = false;
                    afterNativeCall();
                });
    }

    boolean isTextSessionActive(long sessionId) {
        return nativeHandle != 0 && activeTextSession == sessionId;
    }

    private void afterNativeCall() {
        if (nativeHandle == 0) {
            return;
        }

        long[] imeCommand;
        while ((imeCommand = WinitNative.nativeTakeImeCommand(nativeHandle)).length != 0) {
            applyImeCommand(imeCommand);
        }
        if (WinitNative.nativeTakeWork(nativeHandle)) {
            scheduleFrame();
        }
        if (WinitNative.nativeExiting(nativeHandle) && !isFinishing()) {
            finish();
        }
    }

    private void applyImeCommand(long[] command) {
        if (command.length < 8) {
            throw new IllegalStateException("invalid IME command from Rust");
        }
        int kind = (int) command[0];
        long sessionId = command[1];
        InputMethodManager manager = getSystemService(InputMethodManager.class);
        if (kind == 1) {
            finishTextMenu();
            activeTextSession = sessionId;
            textHint = (int) command[2];
            textPurpose = (int) command[3];
            surfaceView.configureTextInput(activeTextSession, textHint, textPurpose);
            surfaceView.setCaretRect(
                    safeInt(command[4]),
                    safeInt(command[5]),
                    safeInt(command[6]),
                    safeInt(command[7]));
            surfaceView.requestFocus();
            manager.restartInput(surfaceView);
            surfaceView.post(() -> manager.showSoftInput(surfaceView, InputMethodManager.SHOW_IMPLICIT));
        } else if (kind == 2 && activeTextSession == sessionId) {
            int oldInputType = surfaceView.currentInputType();
            textHint = (int) command[2];
            textPurpose = (int) command[3];
            surfaceView.configureTextInput(activeTextSession, textHint, textPurpose);
            surfaceView.setCaretRect(
                    safeInt(command[4]),
                    safeInt(command[5]),
                    safeInt(command[6]),
                    safeInt(command[7]));
            if (oldInputType != surfaceView.currentInputType()) {
                manager.restartInput(surfaceView);
            } else {
                long[] state = WinitNative.nativeTextState(nativeHandle, activeTextSession);
                if (state.length >= 6) {
                    manager.updateSelection(
                            surfaceView,
                            safeInt(state[2]),
                            safeInt(state[3]),
                            safeInt(state[4]),
                            safeInt(state[5]));
                }
            }
            if (inputMenu != null) {
                inputMenu.invalidateContentRect();
            }
        } else if (kind == 3 && activeTextSession == sessionId) {
            finishTextMenu();
            manager.hideSoftInputFromWindow(surfaceView.getWindowToken(), 0);
            activeTextSession = 0;
            surfaceView.configureTextInput(0, 0, 0);
            manager.restartInput(surfaceView);
        } else if (kind == 4 && activeTextSession == sessionId) {
            showTextMenu(sessionId);
        } else if (kind == 5) {
            surfaceView.performHapticFeedback(HapticFeedbackConstants.LONG_PRESS);
        } else if (kind == 6) {
            moveTaskToBack(true);
        }
    }

    void updateTextSelection(
            long sessionId,
            int anchor,
            int focus,
            int compositionStart,
            int compositionEnd) {
        if (nativeHandle == 0 || activeTextSession != sessionId) {
            return;
        }
        getSystemService(InputMethodManager.class)
                .updateSelection(surfaceView, anchor, focus, compositionStart, compositionEnd);
        if (inputMenu != null && inputMenuSession == sessionId) {
            inputMenu.invalidate();
            inputMenu.invalidateContentRect();
        }
    }

    void textConnectionClosed(WinitInputConnection connection) {
        surfaceView.clearInputConnection(connection);
        if (inputMenuSession == connection.sessionId()) {
            finishTextMenu();
        }
    }

    private void showTextMenu(long sessionId) {
        WinitInputConnection connection = surfaceView.inputConnection(sessionId);
        if (connection == null) {
            return;
        }
        finishTextMenu();
        inputMenuSession = sessionId;
        inputMenu =
                surfaceView.startActionMode(
                        new ActionMode.Callback2() {
                            @Override
                            public boolean onCreateActionMode(ActionMode mode, Menu menu) {
                                populateTextMenu(menu, connection);
                                return menu.size() > 0;
                            }

                            @Override
                            public boolean onPrepareActionMode(ActionMode mode, Menu menu) {
                                menu.clear();
                                populateTextMenu(menu, connection);
                                return true;
                            }

                            @Override
                            public boolean onActionItemClicked(ActionMode mode, MenuItem item) {
                                boolean handled =
                                        connection.performContextMenuAction(item.getItemId());
                                if (!handled) {
                                    return false;
                                }
                                if (item.getItemId() == android.R.id.selectAll) {
                                    mode.invalidate();
                                    mode.invalidateContentRect();
                                } else {
                                    mode.finish();
                                }
                                return true;
                            }

                            @Override
                            public void onDestroyActionMode(ActionMode mode) {
                                if (inputMenu == mode) {
                                    inputMenu = null;
                                    inputMenuSession = -1;
                                }
                            }

                            @Override
                            public void onGetContentRect(ActionMode mode, View view, Rect outRect) {
                                surfaceView.copyCaretRect(outRect);
                            }
                        },
                        ActionMode.TYPE_FLOATING);
        if (inputMenu == null) {
            inputMenuSession = -1;
        }
    }

    private void populateTextMenu(Menu menu, WinitInputConnection connection) {
        boolean selected = connection.hasSelection();
        if (selected && !connection.isSecure()) {
            addTextMenuItem(menu, android.R.id.cut, android.R.string.cut);
            addTextMenuItem(menu, android.R.id.copy, android.R.string.copy);
        }
        ClipboardManager clipboard = getSystemService(ClipboardManager.class);
        if (clipboard.hasPrimaryClip()) {
            addTextMenuItem(menu, android.R.id.paste, android.R.string.paste);
        }
        if (connection.hasText()) {
            addTextMenuItem(menu, android.R.id.selectAll, android.R.string.selectAll);
        }
    }

    private static void addTextMenuItem(Menu menu, int id, int title) {
        menu.add(Menu.NONE, id, Menu.NONE, title).setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS);
    }

    private void finishTextMenu() {
        ActionMode menu = inputMenu;
        inputMenu = null;
        inputMenuSession = -1;
        if (menu != null) {
            menu.finish();
        }
    }

    private static int safeInt(long value) {
        if (value < Integer.MIN_VALUE) {
            return Integer.MIN_VALUE;
        }
        if (value > Integer.MAX_VALUE) {
            return Integer.MAX_VALUE;
        }
        return (int) value;
    }

    private void scheduleFrame() {
        if (framePosted || nativeHandle == 0) {
            return;
        }
        framePosted = true;
        Choreographer.getInstance().postFrameCallback(this);
    }

    private void installInsetsListener() {
        surfaceView.setOnApplyWindowInsetsListener(
                (view, insets) -> {
                    reportWindowInsets(insets);
                    return insets;
                });
        if (Build.VERSION.SDK_INT >= 30) {
            surfaceView.setWindowInsetsAnimationCallback(
                    new WindowInsetsAnimation.Callback(
                            WindowInsetsAnimation.Callback.DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
                        @Override
                        public WindowInsets onProgress(
                                WindowInsets insets,
                                List<WindowInsetsAnimation> runningAnimations) {
                            reportWindowInsets(insets);
                            return insets;
                        }
                    });
        }
        surfaceView.requestApplyInsets();
    }

    @SuppressWarnings("deprecation")
    private void reportWindowInsets(WindowInsets insets) {
        if (nativeHandle == 0 || insets == null) {
            return;
        }
        if (Build.VERSION.SDK_INT >= 30) {
            Insets bars =
                    insets.getInsets(
                            WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            Insets ime = insets.getInsets(WindowInsets.Type.ime());
            Insets safe = Insets.max(bars, ime);
            WinitNative.nativeInsets(
                    nativeHandle,
                    safe.left,
                    safe.top,
                    safe.right,
                    safe.bottom,
                    insets.isVisible(WindowInsets.Type.ime()));
        } else {
            WinitNative.nativeInsets(
                    nativeHandle,
                    insets.getSystemWindowInsetLeft(),
                    insets.getSystemWindowInsetTop(),
                    insets.getSystemWindowInsetRight(),
                    insets.getSystemWindowInsetBottom(),
                    false);
        }
        afterNativeCall();
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (nativeHandle != 0
                && event.getKeyCode() != KeyEvent.KEYCODE_BACK
                && (event.getAction() == KeyEvent.ACTION_DOWN
                        || event.getAction() == KeyEvent.ACTION_UP)) {
            boolean handled =
                    WinitNative.nativeKey(
                            nativeHandle,
                            event.getAction() == KeyEvent.ACTION_DOWN,
                            event.getDeviceId(),
                            event.getKeyCode(),
                            Integer.toUnsignedLong(event.getUnicodeChar(event.getMetaState())),
                            Integer.toUnsignedLong(event.getUnicodeChar(0)),
                            event.getRepeatCount() > 0);
            afterNativeCall();
            if (handled) {
                return true;
            }
        }
        return super.dispatchKeyEvent(event);
    }

    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        if (nativeHandle != 0) {
            boolean handled = WinitNative.nativeBack(nativeHandle);
            afterNativeCall();
            if (handled) {
                return;
            }
        }
        super.onBackPressed();
    }

    static final class WinitSurfaceView extends SurfaceView {
        private final WinitActivity activity;
        private long nativeHandle;
        private long textSession;
        private int textHint;
        private int textPurpose;
        private int inputType = InputType.TYPE_CLASS_TEXT;
        private final Rect caretRect = new Rect(0, 0, 1, 1);
        private WinitInputConnection inputConnection;

        WinitSurfaceView(WinitActivity activity) {
            super(activity);
            this.activity = activity;
        }

        void setNativeHandle(long nativeHandle) {
            this.nativeHandle = nativeHandle;
        }

        void configureTextInput(long session, int hint, int purpose) {
            textSession = session;
            textHint = hint;
            textPurpose = purpose;
            inputType = inputType(hint, purpose);
        }

        int currentInputType() {
            return inputType;
        }

        void setCaretRect(int left, int top, int right, int bottom) {
            caretRect.set(left, top, Math.max(left + 1, right), Math.max(top + 1, bottom));
        }

        void copyCaretRect(Rect destination) {
            destination.set(caretRect);
            int width = Math.max(1, getWidth());
            int height = Math.max(1, getHeight());
            if (destination.right < 0) {
                destination.offset(-destination.right, 0);
            } else if (destination.left > width) {
                destination.offset(width - destination.left, 0);
            }
            if (destination.bottom < 0) {
                destination.offset(0, -destination.bottom);
            } else if (destination.top > height) {
                destination.offset(0, height - destination.top);
            }
        }

        WinitInputConnection inputConnection(long sessionId) {
            return inputConnection != null && inputConnection.sessionId() == sessionId
                    ? inputConnection
                    : null;
        }

        void clearInputConnection(WinitInputConnection connection) {
            if (inputConnection == connection) {
                inputConnection = null;
            }
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return textSession != 0;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
            if (textSession == 0 || nativeHandle == 0) {
                return null;
            }
            outAttrs.inputType = inputType;
            outAttrs.imeOptions =
                    (textHint & 0x200) != 0
                            ? EditorInfo.IME_FLAG_NO_ENTER_ACTION
                            : EditorInfo.IME_ACTION_DONE;
            long[] state = WinitNative.nativeTextState(nativeHandle, textSession);
            activity.afterTextInputCall();
            if (state.length >= 6) {
                outAttrs.initialSelStart = (int) state[2];
                outAttrs.initialSelEnd = (int) state[3];
            }
            boolean multiline = (inputType & InputType.TYPE_TEXT_FLAG_MULTI_LINE) != 0;
            boolean secure = textPurpose == 1 || textPurpose == 7;
            if (Build.VERSION.SDK_INT >= 34) {
                inputConnection =
                        new WinitInputConnectionApi34(
                                activity,
                                this,
                                nativeHandle,
                                textSession,
                                multiline,
                                secure);
            } else if (Build.VERSION.SDK_INT >= 33) {
                inputConnection =
                        new WinitInputConnectionApi33(
                                activity,
                                this,
                                nativeHandle,
                                textSession,
                                multiline,
                                secure);
            } else if (Build.VERSION.SDK_INT >= 31) {
                inputConnection =
                        new WinitInputConnectionApi31(
                                activity,
                                this,
                                nativeHandle,
                                textSession,
                                multiline,
                                secure);
            } else {
                inputConnection =
                        new WinitInputConnection(
                                activity,
                                this,
                                nativeHandle,
                                textSession,
                                multiline,
                                secure);
            }
            return inputConnection;
        }

        @Override
        public boolean onTouchEvent(MotionEvent event) {
            if (nativeHandle == 0) {
                return false;
            }

            boolean handled = false;
            int masked = event.getActionMasked();
            int actionIndex = event.getActionIndex();
            if (masked == MotionEvent.ACTION_MOVE) {
                for (int history = 0; history < event.getHistorySize(); history++) {
                    long time = historicalEventTimeNanos(event, history);
                    for (int pointer = 0; pointer < event.getPointerCount(); pointer++) {
                        handled |=
                                WinitNative.nativePointer(
                                        nativeHandle,
                                        WinitNative.POINTER_MOVE,
                                        time,
                                        event.getDeviceId(),
                                        event.getPointerId(pointer),
                                        event.getHistoricalX(pointer, history),
                                        event.getHistoricalY(pointer, history),
                                        event.getHistoricalPressure(pointer, history));
                    }
                }
                long time = eventTimeNanos(event);
                for (int pointer = 0; pointer < event.getPointerCount(); pointer++) {
                    handled |=
                            WinitNative.nativePointer(
                                    nativeHandle,
                                    WinitNative.POINTER_MOVE,
                                    time,
                                    event.getDeviceId(),
                                    event.getPointerId(pointer),
                                    event.getX(pointer),
                                    event.getY(pointer),
                                    event.getPressure(pointer));
                }
            } else if (masked == MotionEvent.ACTION_DOWN
                    || masked == MotionEvent.ACTION_POINTER_DOWN
                    || masked == MotionEvent.ACTION_UP
                    || masked == MotionEvent.ACTION_POINTER_UP) {
                int action =
                        (masked == MotionEvent.ACTION_DOWN
                                        || masked == MotionEvent.ACTION_POINTER_DOWN)
                                ? WinitNative.POINTER_DOWN
                                : WinitNative.POINTER_UP;
                handled =
                        WinitNative.nativePointer(
                                nativeHandle,
                                action,
                                eventTimeNanos(event),
                                event.getDeviceId(),
                                event.getPointerId(actionIndex),
                                event.getX(actionIndex),
                                event.getY(actionIndex),
                                event.getPressure(actionIndex));
            } else if (masked == MotionEvent.ACTION_CANCEL) {
                long time = eventTimeNanos(event);
                for (int pointer = 0; pointer < event.getPointerCount(); pointer++) {
                    handled |=
                            WinitNative.nativePointer(
                                    nativeHandle,
                                    WinitNative.POINTER_CANCEL,
                                    time,
                                    event.getDeviceId(),
                                    event.getPointerId(pointer),
                                    event.getX(pointer),
                                    event.getY(pointer),
                                    event.getPressure(pointer));
                }
            }

            activity.afterPointerCall();
            return handled;
        }

        private static int inputType(int hint, int purpose) {
            int type;
            switch (purpose) {
                case 1:
                    type =
                            InputType.TYPE_CLASS_TEXT
                                    | InputType.TYPE_TEXT_VARIATION_PASSWORD
                                    | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
                    break;
                case 2:
                    type =
                            InputType.TYPE_CLASS_TEXT
                                    | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                                    | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
                    break;
                case 3:
                    type =
                            InputType.TYPE_CLASS_NUMBER
                                    | InputType.TYPE_NUMBER_FLAG_DECIMAL
                                    | InputType.TYPE_NUMBER_FLAG_SIGNED;
                    break;
                case 4:
                    type = InputType.TYPE_CLASS_PHONE;
                    break;
                case 5:
                    type = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI;
                    break;
                case 6:
                    type = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS;
                    break;
                case 7:
                    type =
                            InputType.TYPE_CLASS_NUMBER
                                    | InputType.TYPE_NUMBER_VARIATION_PASSWORD;
                    break;
                case 8:
                    type = InputType.TYPE_CLASS_DATETIME | InputType.TYPE_DATETIME_VARIATION_DATE;
                    break;
                case 9:
                    type = InputType.TYPE_CLASS_DATETIME | InputType.TYPE_DATETIME_VARIATION_TIME;
                    break;
                case 10:
                    type = InputType.TYPE_CLASS_DATETIME;
                    break;
                default:
                    type = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_NORMAL;
                    break;
            }

            if ((type & InputType.TYPE_MASK_CLASS) == InputType.TYPE_CLASS_TEXT) {
                if ((hint & 0x2) != 0) {
                    type |= InputType.TYPE_TEXT_FLAG_AUTO_CORRECT;
                }
                if ((hint & 0x4) != 0) {
                    type |= InputType.TYPE_TEXT_FLAG_CAP_SENTENCES;
                }
                if ((hint & 0x20) != 0) {
                    type |= InputType.TYPE_TEXT_FLAG_CAP_WORDS;
                }
                if ((hint & 0x200) != 0) {
                    type |= InputType.TYPE_TEXT_FLAG_MULTI_LINE;
                }
                if ((hint & (0x40 | 0x80)) != 0) {
                    type |= InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
                }
            }
            return type;
        }

        private static long eventTimeNanos(MotionEvent event) {
            if (Build.VERSION.SDK_INT >= 29) {
                return event.getEventTimeNanos();
            }
            return event.getEventTime() * 1_000_000L;
        }

        private static long historicalEventTimeNanos(MotionEvent event, int position) {
            if (Build.VERSION.SDK_INT >= 29) {
                return event.getHistoricalEventTimeNanos(position);
            }
            return event.getHistoricalEventTime(position) * 1_000_000L;
        }
    }
}
