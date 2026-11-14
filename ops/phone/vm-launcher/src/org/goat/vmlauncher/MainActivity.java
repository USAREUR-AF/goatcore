// SPDX-License-Identifier: Apache-2.0
package org.goat.vmlauncher;

import android.app.Activity;
import android.os.Bundle;

/** Opening the app once takes it out of Android's stopped state, so it receives the boot broadcast. */
public final class MainActivity extends Activity {
    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        Launch.terminal(this, "opened by hand");
        finish();
    }
}
