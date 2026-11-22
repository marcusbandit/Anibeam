package com.marcusrosado.anibeam;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;

/**
 * Where an anibeam:// link from a browser lands (#1406). The app's activity is a stock
 * NativeActivity, which never hands a new intent to the native side, so a link tapped while
 * the app runs would be lost. This one keeps the link, brings the app's activity forward
 * (singleTask starts it or reuses the one running) and goes away. The app takes the link
 * with take() when it comes to the front, and decides there what the link may do: nothing
 * here reads it.
 */
public class LinkActivity extends Activity {
    private static String pending;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        Uri data = getIntent() == null ? null : getIntent().getData();
        if (data != null) {
            synchronized (LinkActivity.class) {
                pending = data.toString();
            }
        }
        Intent open = new Intent();
        open.setClassName(getPackageName(), "android.app.NativeActivity");
        open.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            startActivity(open);
        } catch (RuntimeException e) {
            // Nothing to show it on; the link stays for the next time the app opens.
        }
        finish();
    }

    /** The link waiting for the app, or null, and forgets it: each link acts once. */
    public static String take(Activity activity) {
        synchronized (LinkActivity.class) {
            String link = pending;
            pending = null;
            return link;
        }
    }
}
