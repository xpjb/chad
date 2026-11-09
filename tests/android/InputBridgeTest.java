package io.github.xpjb.chad.test;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.ClipboardManager;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.ActionMode;
import android.view.InputDevice;
import android.view.MotionEvent;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.widget.EditText;
import io.github.xpjb.chad.ChadActivity;
import java.lang.reflect.Field;

public final class InputBridgeTest extends Instrumentation {
    private static native int reports();
    private static native int touches();

    private ChadActivity activity;
    private EditText input;

    private void state(long generation, String text, int anchor, int focus, boolean secret) {
        android.util.DisplayMetrics size = activity.getResources().getDisplayMetrics();
        activity.chadSetTextInput(1, generation, 0, text, anchor, focus,
            20, size.heightPixels / 2.0f, size.widthPixels - 40, 100,
            60, size.heightPixels / 2.0f + 20, 60, 32,
            true, secret);
    }

    private Object field(String name) throws Exception {
        Field field = ChadActivity.class.getDeclaredField(name);
        field.setAccessible(true);
        return field.get(activity);
    }

    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }

    @Override public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        start();
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            Intent launch = new Intent().setComponent(new ComponentName(
                getTargetContext(), ChadActivity.class)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            activity = (ChadActivity)startActivitySync(launch);
            runOnMainSync(() -> {
                state(1, "one two three", 4, 7, false);
                try {
                    input = (EditText)field("input");
                } catch (Exception error) {
                    throw new RuntimeException(error);
                }
            });
            waitForIdleSync();
            SystemClock.sleep(300);

            android.graphics.Bitmap beforeMenu = getUiAutomation().takeScreenshot();
            runOnMainSync(activity::chadShowTextMenu);
            waitForIdleSync();
            SystemClock.sleep(400);
            ActionMode menu = (ActionMode)field("inputMenu");
            check(menu != null && menu.getType() == ActionMode.TYPE_FLOATING,
                "Floating clipboard toolbar did not open");
            android.graphics.Bitmap shownMenu = getUiAutomation().takeScreenshot();
            int changed = 0;
            for (int y = beforeMenu.getHeight() / 4; y < beforeMenu.getHeight() * 3 / 4; y++) {
                for (int x = 0; x < beforeMenu.getWidth(); x++) {
                    if (beforeMenu.getPixel(x, y) != shownMenu.getPixel(x, y)) changed++;
                }
            }
            check(changed > 500, "Toolbar exists but is not visibly painted");
            beforeMenu.recycle();
            shownMenu.recycle();

            int beforeTouches = touches();
            long now = SystemClock.uptimeMillis();
            float touchY = activity.getResources().getDisplayMetrics().heightPixels - 200;
            MotionEvent down = MotionEvent.obtain(now, now, MotionEvent.ACTION_DOWN, 60, touchY, 0);
            MotionEvent up = MotionEvent.obtain(now, now + 30, MotionEvent.ACTION_UP, 60, touchY, 0);
            down.setSource(InputDevice.SOURCE_TOUCHSCREEN);
            up.setSource(InputDevice.SOURCE_TOUCHSCREEN);
            sendPointerSync(down);
            sendPointerSync(up);
            down.recycle();
            up.recycle();
            SystemClock.sleep(150);
            check(touches() >= beforeTouches + 2,
                "Toolbar swallowed the first underlying handle gesture");

            runOnMainSync(() -> menu.getMenu().performIdentifierAction(android.R.id.copy, 0));
            runOnMainSync(() -> {
                ClipboardManager clipboard = (ClipboardManager)activity.getSystemService(
                    Context.CLIPBOARD_SERVICE);
                check("two".contentEquals(clipboard.getPrimaryClip().getItemAt(0).getText()),
                    "Copy used the wrong selection");
                activity.chadShowTextMenu();
            });
            runOnMainSync(() -> {
                try {
                    ActionMode select = (ActionMode)field("inputMenu");
                    select.getMenu().performIdentifierAction(android.R.id.selectAll, 0);
                    check(input.getSelectionStart() == 0 && input.getSelectionEnd() == 13,
                        "Select All did not update the Editable");
                } catch (Exception error) {
                    throw new RuntimeException(error);
                }
            });

            final InputConnection[] connection = new InputConnection[1];
            runOnMainSync(() -> {
                state(2, "abc", 3, 3, false);
                try {
                    input = (EditText)field("input");
                } catch (Exception error) {
                    throw new RuntimeException(error);
                }
                connection[0] = input.onCreateInputConnection(new EditorInfo());
            });
            waitForIdleSync();
            int beforeReports = reports();
            runOnMainSync(() -> {
                check(connection[0].beginBatchEdit(), "Batch edit did not start");
                check(connection[0].commitText("x", 1), "First rapid commit failed");
                check(connection[0].commitText("y", 1), "Second rapid commit failed");
            });
            waitForIdleSync();
            SystemClock.sleep(100);
            check(reports() == beforeReports, "Batch edit reported an incomplete snapshot");
            runOnMainSync(() -> connection[0].endBatchEdit());
            waitForIdleSync();
            SystemClock.sleep(100);
            check(reports() == beforeReports + 1, "Batch edit did not report exactly once");
            runOnMainSync(() -> {
                check("abcxy".contentEquals(input.getText()), "Rapid commits lost text");
                state(2, "abc", 3, 3, false);
                check("abcxy".contentEquals(input.getText()),
                    "A stale same-generation frame overwrote IME text");
            });

            runOnMainSync(() -> {
                state(3, "reset", 5, 5, false);
                try {
                    input = (EditText)field("input");
                } catch (Exception error) {
                    throw new RuntimeException(error);
                }
                check(!connection[0].commitText("STALE", 1),
                    "An old IME generation remained active");
                check("reset".contentEquals(input.getText()),
                    "An old IME generation changed the replacement input");
                state(4, "secret", 0, 6, true);
                activity.chadShowTextMenu();
            });
            runOnMainSync(() -> {
                try {
                    ActionMode secret = (ActionMode)field("inputMenu");
                    check(!secret.getMenu().findItem(android.R.id.copy).isVisible(),
                        "Secret copy must stay disabled");
                    check(!secret.getMenu().findItem(android.R.id.cut).isVisible(),
                        "Secret cut must stay disabled");
                    activity.chadClearTextInput();
                    check(field("inputMenu") == null && field("input") == null,
                        "Detached fields retained input UI");
                } catch (Exception error) {
                    throw new RuntimeException(error);
                }
            });

            result.putString("stream",
                "PASS: toolbar, stale-generation fence, batch commits and same-generation IME ownership\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", "FAIL: " + android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
