// SPDX-License-Identifier: Apache-2.0
package org.goat.vmlauncher;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/** Runs once the phone has booted and its storage is unlocked. */
public final class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context c, Intent intent) {
        Launch.terminal(c, String.valueOf(intent.getAction()));
    }
}
