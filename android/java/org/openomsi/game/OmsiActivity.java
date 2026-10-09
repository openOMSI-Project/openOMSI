package org.openomsi.game;

import android.Manifest;
import android.app.NativeActivity;
import android.app.PendingIntent;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.os.VibrationEffect;
import android.os.Vibrator;
import android.provider.Settings;
import android.view.View;
import android.view.WindowManager;

import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * openOMSI's activity: the game itself is native code (libopenomsi_game.so, run by
 * NativeActivity). This adds only what NativeActivity lacks: the whole screen without the
 * system bars, the screen kept on while playing, access to the shared storage (where the
 * copy of OMSI 2 and the mods are), the vibration of the on-screen buttons, and installing
 * an update (the launcher downloads the APK from the GitHub release; the system's package
 * installer asks the player and replaces the app).
 */
public class OmsiActivity extends NativeActivity {
    private boolean askedStorage = false;

    /** The package installer's answer, polled by the native side (see android.rs):
     * 0 nothing, 1 asking the player, 2 installed, 3 cancelled, 4 failed, 5 waiting for
     * "Install unknown apps", 6 that refused. */
    private static volatile int installStatus = 0;
    private static volatile String installMessage = "";
    private static final String ACTION_INSTALLED = "org.openomsi.game.INSTALL_STATUS";
    /** An APK waiting for the "Install unknown apps" permission. */
    private String pendingApk = null;
    private boolean askedInstallPermission = false;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        if (Build.VERSION.SDK_INT >= 28) {
            // drawn beside a camera cut-out as well, the controls keep away from the edges
            WindowManager.LayoutParams lp = getWindow().getAttributes();
            lp.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            getWindow().setAttributes(lp);
        }
        immersive();
        askForStorage();
        // started by the package installer after an update: nothing more to do
        Intent i = getIntent();
        if (i != null && ACTION_INSTALLED.equals(i.getAction())) {
            installResult(i);
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        if (intent != null && ACTION_INSTALLED.equals(intent.getAction())) {
            installResult(intent);
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        // back from "Install unknown apps"
        if (pendingApk != null && askedInstallPermission) {
            String apk = pendingApk;
            pendingApk = null;
            askedInstallPermission = false;
            if (Build.VERSION.SDK_INT < 26 || getPackageManager().canRequestPackageInstalls()) {
                startInstall(apk);
            } else {
                installStatus = 6;
            }
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            immersive();
        }
    }

    @SuppressWarnings("deprecation")
    private void immersive() {
        View decor = getWindow().getDecorView();
        decor.setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                | View.SYSTEM_UI_FLAG_FULLSCREEN
                | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN);
    }

    /** Whether the app may read and write the shared storage (the OMSI 2 folder, the mods). */
    public boolean hasStorage() {
        if (Build.VERSION.SDK_INT >= 30) {
            return Environment.isExternalStorageManager();
        }
        return checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED;
    }

    private void askForStorage() {
        if (askedStorage || hasStorage()) {
            return;
        }
        askedStorage = true;
        if (Build.VERSION.SDK_INT >= 30) {
            // "All files access": OMSI 2 and its mods are thousands of files the game reads
            // by path, which only this permission allows
            try {
                Intent i = new Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:" + getPackageName()));
                startActivity(i);
            } catch (Exception e) {
                try {
                    startActivity(new Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION));
                } catch (Exception ignored) {
                }
            }
        } else {
            requestPermissions(new String[] {Manifest.permission.READ_EXTERNAL_STORAGE, Manifest.permission.WRITE_EXTERNAL_STORAGE}, 1);
        }
    }

    /** A short buzz (called from the native side for the on-screen buttons). */
    @SuppressWarnings("deprecation")
    public void vibrate(int ms) {
        try {
            Vibrator v = (Vibrator) getSystemService(VIBRATOR_SERVICE);
            if (v == null || !v.hasVibrator()) {
                return;
            }
            if (Build.VERSION.SDK_INT >= 26) {
                v.vibrate(VibrationEffect.createOneShot(ms, VibrationEffect.DEFAULT_AMPLITUDE));
            } else {
                v.vibrate(ms);
            }
        } catch (Exception ignored) {
        }
    }

    public int getInstallStatus() {
        return installStatus;
    }

    public String getInstallMessage() {
        return installMessage;
    }

    /** Install the update `path` (called from the native side, any thread). */
    public void installApk(final String path) {
        installStatus = 1;
        installMessage = "";
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                startInstall(path);
            }
        });
    }

    private void startInstall(String path) {
        if (Build.VERSION.SDK_INT >= 26 && !getPackageManager().canRequestPackageInstalls()) {
            // the player allows openOMSI to install apps first, then comes back here
            pendingApk = path;
            askedInstallPermission = true;
            installStatus = 5;
            try {
                startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + getPackageName())));
            } catch (Exception e) {
                pendingApk = null;
                installStatus = 6;
            }
            return;
        }
        PackageInstaller.Session session = null;
        try {
            PackageInstaller pi = getPackageManager().getPackageInstaller();
            PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
            params.setAppPackageName(getPackageName());
            int id = pi.createSession(params);
            session = pi.openSession(id);
            File f = new File(path);
            InputStream in = new FileInputStream(f);
            OutputStream out = session.openWrite("openomsi.apk", 0, f.length());
            byte[] buf = new byte[1 << 16];
            int n;
            while ((n = in.read(buf)) > 0) {
                out.write(buf, 0, n);
            }
            session.fsync(out);
            out.close();
            in.close();
            // the answer comes back to this activity (and, once installed, starts the new app)
            Intent answer = new Intent(this, OmsiActivity.class).setAction(ACTION_INSTALLED);
            int flags = PendingIntent.FLAG_UPDATE_CURRENT;
            if (Build.VERSION.SDK_INT >= 31) {
                flags |= PendingIntent.FLAG_MUTABLE;
            }
            PendingIntent pending = PendingIntent.getActivity(this, 7, answer, flags);
            session.commit(pending.getIntentSender());
            session.close();
            installStatus = 1;
        } catch (Exception e) {
            if (session != null) {
                session.abandon();
            }
            installMessage = String.valueOf(e.getMessage());
            installStatus = 4;
        }
    }

    @SuppressWarnings("deprecation")
    private void installResult(Intent intent) {
        int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
        switch (status) {
            case PackageInstaller.STATUS_PENDING_USER_ACTION: {
                // the system's "Do you want to update this app?"
                Intent confirm = (Intent) intent.getParcelableExtra(Intent.EXTRA_INTENT);
                if (confirm != null) {
                    try {
                        startActivity(confirm);
                        installStatus = 1;
                    } catch (Exception e) {
                        installMessage = String.valueOf(e.getMessage());
                        installStatus = 4;
                    }
                }
                break;
            }
            case PackageInstaller.STATUS_SUCCESS:
                installStatus = 2;
                break;
            case PackageInstaller.STATUS_FAILURE_ABORTED:
                installStatus = 3;
                break;
            case PackageInstaller.STATUS_FAILURE_CONFLICT:
            case PackageInstaller.STATUS_FAILURE_INCOMPATIBLE:
                // signed with another key than the installed app (a build of one's own)
                installMessage = "this openOMSI was installed from a build with another signature than the GitHub releases. Uninstall it once and install the APK from github.com/openOMSI-org/openOMSI - updates work from then on.";
                installStatus = 4;
                break;
            default: {
                String msg = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
                installMessage = msg != null ? msg : ("status " + status);
                installStatus = 4;
            }
        }
    }

    /** A web page in the browser (the release on GitHub). */
    public void openUrl(String url) {
        try {
            Intent i = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            startActivity(i);
        } catch (Exception ignored) {
        }
    }
}
