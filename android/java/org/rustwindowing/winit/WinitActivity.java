package org.rustwindowing.winit;

import android.app.Activity;
import android.os.Build;
import android.os.Bundle;
import android.view.Choreographer;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.text.InputType;
import android.view.WindowInsets;

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
        if (command.length < 4) {
            throw new IllegalStateException("invalid IME command from Rust");
        }
        int kind = (int) command[0];
        long sessionId = command[1];
        InputMethodManager manager = getSystemService(InputMethodManager.class);
        if (kind == 1) {
            activeTextSession = sessionId;
            textHint = (int) command[2];
            textPurpose = (int) command[3];
            surfaceView.configureTextInput(activeTextSession, textHint, textPurpose);
            surfaceView.requestFocus();
            manager.restartInput(surfaceView);
            surfaceView.post(() -> manager.showSoftInput(surfaceView, InputMethodManager.SHOW_IMPLICIT));
        } else if (kind == 2 && activeTextSession == sessionId) {
            int oldInputType = surfaceView.currentInputType();
            textHint = (int) command[2];
            textPurpose = (int) command[3];
            surfaceView.configureTextInput(activeTextSession, textHint, textPurpose);
            if (oldInputType != surfaceView.currentInputType()) {
                manager.restartInput(surfaceView);
            } else {
                long[] state = WinitNative.nativeTextState(nativeHandle, activeTextSession);
                if (state.length >= 6) {
                    manager.updateSelection(
                            surfaceView,
                            (int) state[2],
                            (int) state[3],
                            (int) state[4],
                            (int) state[5]);
                }
            }
        } else if (kind == 3 && activeTextSession == sessionId) {
            manager.hideSoftInputFromWindow(surfaceView.getWindowToken(), 0);
            activeTextSession = 0;
            surfaceView.configureTextInput(0, 0, 0);
            manager.restartInput(surfaceView);
        }
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
                    if (nativeHandle != 0) {
                        if (Build.VERSION.SDK_INT >= 30) {
                            android.graphics.Insets bars =
                                    insets.getInsets(
                                            WindowInsets.Type.systemBars()
                                                    | WindowInsets.Type.displayCutout());
                            WinitNative.nativeInsets(
                                    nativeHandle, bars.left, bars.top, bars.right, bars.bottom);
                        } else {
                            WinitNative.nativeInsets(
                                    nativeHandle,
                                    insets.getSystemWindowInsetLeft(),
                                    insets.getSystemWindowInsetTop(),
                                    insets.getSystemWindowInsetRight(),
                                    insets.getSystemWindowInsetBottom());
                        }
                        afterNativeCall();
                    }
                    return insets;
                });
        surfaceView.requestApplyInsets();
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

    private static final class WinitSurfaceView extends SurfaceView {
        private final WinitActivity activity;
        private long nativeHandle;
        private long textSession;
        private int textHint;
        private int textPurpose;
        private int inputType = InputType.TYPE_CLASS_TEXT;

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
            return new WinitInputConnection(
                    activity,
                    this,
                    nativeHandle,
                    textSession,
                    (inputType & InputType.TYPE_TEXT_FLAG_MULTI_LINE) != 0);
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
