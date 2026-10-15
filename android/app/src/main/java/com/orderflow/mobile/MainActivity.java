package com.orderflow.mobile;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.os.Bundle;
import android.os.Process;
import android.util.Log;
import java.io.File;
import java.io.FileWriter;
import java.io.PrintWriter;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.WindowManager;

import androidx.activity.OnBackPressedCallback;
import androidx.annotation.NonNull;
import androidx.core.graphics.Insets;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsAnimationCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import java.util.List;

import com.google.androidgamesdk.GameActivity;

/**
 * The one activity of the app. The terminal itself is Rust (libof_mobile.so,
 * started by GameActivity's glue in {@code android_main}); this class only
 * does what Rust cannot reach through winit 0.30 (crates/of-mobile/src/android.rs):
 *
 * <ul>
 *   <li>keeps one activity for the process: back closes what the terminal
 *       has open and with nothing open moves the app to the background,
 *       configuration changes do not recreate it (manifest), and a real
 *       destroy ends the process — winit cannot start a second event loop in a
 *       process, and its Android backend never ends the first one on destroy;</li>
 *   <li>hands the system-bar, camera-cutout and keyboard insets to Rust;</li>
 *   <li>hands a hovering S Pen (or a DeX mouse) to Rust: winit drops hover events;</li>
 *   <li>puts what the terminal copies on the system clipboard ({@link #copyToClipboard}).</li>
 * </ul>
 */
public class MainActivity extends GameActivity {
    private static final String TAG = "OrderFlow";

    /** Whether an activity was created in this process before. */
    private static boolean started;

    private native void nativeInsets(int left, int top, int right, int bottom);

    private native void nativeHover(boolean gone, float x, float y);

    /** True when the terminal has something open and closes it itself. */
    private native boolean nativeBack();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        writeJavaCrashesToFile();
        if (started) {
            // The process outlived its activity: start over in a fresh one.
            Log.w(TAG, "activity recreated in a live process: restarting");
            Process.killProcess(Process.myPid());
        }
        started = true;
        super.onCreate(savedInstanceState);

        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        hideSystemBars();

        View decor = getWindow().getDecorView();
        // A layout listener and an animation callback, not an insets listener:
        // GameActivity may install its own. With adjustNothing the keyboard
        // relayouts nothing, so its height comes through the animation callback.
        decor.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> sendInsets());
        ViewCompat.setWindowInsetsAnimationCallback(decor, new WindowInsetsAnimationCompat.Callback(
                WindowInsetsAnimationCompat.Callback.DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
            @NonNull
            @Override
            public WindowInsetsCompat onProgress(@NonNull WindowInsetsCompat insets,
                    @NonNull List<WindowInsetsAnimationCompat> running) {
                return insets;
            }

            @Override
            public void onEnd(@NonNull WindowInsetsAnimationCompat animation) {
                sendInsets();
            }
        });

        getOnBackPressedDispatcher().addCallback(this, new OnBackPressedCallback(true) {
            @Override
            public void handleOnBackPressed() {
                back();
            }
        });
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
            sendInsets();
        }
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        // Back never reaches winit (it would end its event loop): the terminal
        // closes a menu, a field's focus, the diagnostics or the crosshair, and
        // with nothing open the app goes to the background.
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
            if (event.getAction() == KeyEvent.ACTION_UP) {
                back();
            }
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent ev) {
        switch (ev.getActionMasked()) {
            case MotionEvent.ACTION_HOVER_ENTER:
            case MotionEvent.ACTION_HOVER_MOVE:
                hover(false, ev.getX(), ev.getY());
                break;
            case MotionEvent.ACTION_HOVER_EXIT:
                hover(true, ev.getX(), ev.getY());
                break;
            default:
                break;
        }
        return super.dispatchGenericMotionEvent(ev);
    }

    @Override
    protected void onDestroy() {
        // GameActivity's onDestroy waits for android_main to return, which
        // winit's Android backend never makes it do — also when a configuration
        // change the manifest does not list recreates the activity: end the
        // process instead, always.
        Log.i(TAG, "activity destroyed: ending the process");
        Process.killProcess(Process.myPid());
        super.onDestroy();
    }

    /**
     * A Java exception before Rust starts logging (a missing native library,
     * a broken JNI call) would show only in logcat, which a phone without a
     * PC does not show: write it next to the Rust log
     * (Android/data/com.orderflow.mobile/files/logs/java-crash.log).
     */
    private void writeJavaCrashesToFile() {
        final Thread.UncaughtExceptionHandler previous = Thread.getDefaultUncaughtExceptionHandler();
        final File dir = new File(getExternalFilesDir(null), "logs");
        Thread.setDefaultUncaughtExceptionHandler((thread, error) -> {
            try {
                dir.mkdirs();
                try (PrintWriter out = new PrintWriter(new FileWriter(new File(dir, "java-crash.log"), true))) {
                    out.println(new java.util.Date() + " thread " + thread.getName());
                    error.printStackTrace(out);
                }
            } catch (Throwable ignored) {
                // Nothing more to do: the original handler still runs.
            }
            if (previous != null) {
                previous.uncaughtException(thread, error);
            }
        });
    }

    private void hideSystemBars() {
        WindowInsetsControllerCompat controller = WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        controller.hide(WindowInsetsCompat.Type.systemBars());
        controller.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    }

    /** GameActivity's own insets listener on its surface: it sees the keyboard come and go. */
    @Override
    public WindowInsetsCompat onApplyWindowInsets(View v, WindowInsetsCompat insets) {
        WindowInsetsCompat result = super.onApplyWindowInsets(v, insets);
        sendInsets(insets);
        return result;
    }

    private void sendInsets() {
        View decor = getWindow().getDecorView();
        if (decor.getRootWindowInsets() == null) {
            return; // Not attached yet; the first layout sends them.
        }
        sendInsets(WindowInsetsCompat.toWindowInsetsCompat(decor.getRootWindowInsets(), decor));
    }

    private void sendInsets(WindowInsetsCompat insets) {
        Insets i = insets.getInsets(WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
        // The keyboard covers the bottom: the terminal lays out above it.
        int bottom = Math.max(i.bottom, insets.getInsets(WindowInsetsCompat.Type.ime()).bottom);
        try {
            nativeInsets(i.left, i.top, i.right, bottom);
        } catch (UnsatisfiedLinkError e) {
            Log.e(TAG, "nativeInsets is missing from libof_mobile.so", e);
        }
    }

    private void back() {
        boolean closed = false;
        try {
            closed = nativeBack();
        } catch (UnsatisfiedLinkError e) {
            Log.e(TAG, "nativeBack is missing from libof_mobile.so", e);
        }
        if (!closed) {
            moveTaskToBack(true);
        }
    }

    /**
     * Called from Rust (android.rs {@code copy_to_clipboard}) with what the
     * terminal copied, such as the chart's «Копировать цену»: egui keeps it
     * inside the app only.
     */
    public void copyToClipboard(String text) {
        runOnUiThread(() -> {
            ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
            if (clipboard != null) {
                clipboard.setPrimaryClip(ClipData.newPlainText("OrderFlow", text));
            }
        });
    }

    private void hover(boolean gone, float x, float y) {
        try {
            nativeHover(gone, x, y);
        } catch (UnsatisfiedLinkError e) {
            Log.e(TAG, "nativeHover is missing from libof_mobile.so", e);
        }
    }
}
