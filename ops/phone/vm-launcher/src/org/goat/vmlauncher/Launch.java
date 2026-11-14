// SPDX-License-Identifier: Apache-2.0
package org.goat.vmlauncher;

import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

/** Opens Android's Linux Terminal app, which starts the Debian VM a goat phone node runs in. */
final class Launch {
    static final String TAG = "goat-vmlauncher";
    private Launch() {}

    static void terminal(Context c, String why) {
        Intent i = new Intent(Intent.ACTION_MAIN);
        i.setComponent(new ComponentName("com.android.virtualization.terminal",
                "com.android.virtualization.terminal.LauncherActivity"));
        i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            c.startActivity(i);
            Log.i(TAG, "opened the Terminal app (" + why + ")");
        } catch (RuntimeException e) {
            Log.e(TAG, "could not open the Terminal app (" + why + "): " + e);
        }
    }
}
