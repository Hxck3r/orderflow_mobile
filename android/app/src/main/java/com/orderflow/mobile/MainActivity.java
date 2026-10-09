package com.orderflow.mobile;

import android.os.Bundle;
import android.os.Process;
import android.util.Log;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.WindowManager;

import androidx.activity.OnBackPressedCallback;
import androidx.core.graphics.Insets;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidx.gamesdk.GameActivity;

/**
 * The one activity of the app. The terminal itself is Rust (libof_mobile.so,
 * started by GameActivity's glue in {@code android_main}); this class only
 * does what Rust cannot reach through winit 0.30 (crates/of-mobile/src/android.rs):
 *
 * <ul>
 *   <li>keeps one activity for the process: back moves the app to the
 *       background, configuration changes do not recreate it (manifest), and a
 *       real destroy ends the process — winit cannot start a second event loop
 *       in a process, and its Android backend never ends the first one on destroy;</li>
 *   <li>hands the system-bar and camera-cutout insets to Rust;</li>
 *   <li>hands a hovering S Pen (or a DeX mouse) to Rust: winit drops hover events.</li>
 * </ul>
 */
public class MainActivity extends GameActivity {
    private static final String TAG = "OrderFlow";

    /** Whether an activity was created in this process before. */
    private static boolean started;

    private native void nativeInsets(int left, int top, int right, int bottom);

    private native void nativeHover(boolean gone, float x, float y);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
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
        // A layout listener, not an insets listener: GameActivity may install its own.
        decor.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> sendInsets());

        getOnBackPressedDispatcher().addCallback(this, new OnBackPressedCallback(true) {
            @Override
            public void handleOnBackPressed() {
                moveTaskToBack(true);
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
        // Back never reaches the terminal (it would end its event loop): the app goes to the background.
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
            if (event.getAction() == KeyEvent.ACTION_UP) {
                moveTaskToBack(true);
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
        if (!isChangingConfigurations()) {
            // GameActivity's onDestroy waits for android_main to return, which
            // winit's Android backend never makes it do: end the process instead.
            Log.i(TAG, "activity destroyed: ending the process");
            Process.killProcess(Process.myPid());
        }
        super.onDestroy();
    }

    private void hideSystemBars() {
        WindowInsetsControllerCompat controller = WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        controller.hide(WindowInsetsCompat.Type.systemBars());
        controller.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    }

    private void sendInsets() {
        View decor = getWindow().getDecorView();
        if (decor.getRootWindowInsets() == null) {
            return; // Not attached yet; the first layout sends them.
        }
        WindowInsetsCompat insets = WindowInsetsCompat.toWindowInsetsCompat(decor.getRootWindowInsets(), decor);
        Insets i = insets.getInsets(WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
        try {
            nativeInsets(i.left, i.top, i.right, i.bottom);
        } catch (UnsatisfiedLinkError e) {
            Log.e(TAG, "nativeInsets is missing from libof_mobile.so", e);
        }
    }

    private void hover(boolean gone, float x, float y) {
        try {
            nativeHover(gone, x, y);
        } catch (UnsatisfiedLinkError e) {
            Log.e(TAG, "nativeHover is missing from libof_mobile.so", e);
        }
    }
}
