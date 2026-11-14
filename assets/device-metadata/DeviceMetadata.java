package org.lightjunction.douyin;

import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Looper;
import dalvik.system.PathClassLoader;
import org.json.JSONObject;

/** Read installed, public package metadata without starting or modifying the app. */
public final class DeviceMetadata {
    private static final String PACKAGE = "com.ss.android.ugc.aweme";

    private static String positiveCode(Bundle metadata, String key) {
        Object value = metadata == null ? null : metadata.get(key);
        if (value == null) throw new IllegalStateException("Missing package version metadata");
        String text = String.valueOf(value);
        if (!text.matches("[1-9][0-9]{0,11}")) {
            throw new IllegalStateException("Invalid package version metadata");
        }
        return text;
    }

    private static String cronetDefaultUserAgent(Context system, ApplicationInfo app,
            PackageInfo info) throws Exception {
        // The symbol is specific to this inspected APK. Do not guess after an update.
        if (!"40.8.0".equals(info.versionName) || info.versionCode != 400801) return null;
        Context packageContext = system.createPackageContext(PACKAGE, 0);
        ClassLoader loader = new PathClassLoader(app.sourceDir, DeviceMetadata.class.getClassLoader());
        Class<?> generator = loader.loadClass("X.1Zev");
        // Populate only this temporary VM's package-info cache with the value
        // already read from PackageManager. The normal cache-miss wrapper needs
        // the main app's reporting SDK; no main-app state is changed here.
        java.lang.reflect.Field version = generator.getDeclaredField("LIZIZ");
        version.setAccessible(true);
        version.setInt(null, info.versionCode);
        Object value = generator.getMethod("LIZ", Context.class).invoke(null, packageContext);
        if (!(value instanceof String) || ((String) value).isEmpty()) {
            throw new IllegalStateException("Missing SDK default User-Agent");
        }
        return (String) value;
    }

    public static void main(String[] args) {
        try {
            Looper.prepareMainLooper();
            Class<?> activityThread = Class.forName("android.app.ActivityThread");
            Object thread = activityThread.getMethod("systemMain").invoke(null);
            Context context = (Context) activityThread.getMethod("getSystemContext").invoke(thread);
            PackageManager manager = context.getPackageManager();
            PackageInfo info = manager.getPackageInfo(PACKAGE, 0);
            ApplicationInfo application = manager.getApplicationInfo(PACKAGE, PackageManager.GET_META_DATA);
            Bundle metadata = application.metaData;
            String versionName = metadata.getString("SS_VERSION_NAME");
            if (versionName == null || !versionName.equals(info.versionName)) {
                throw new IllegalStateException("Package version names disagree");
            }
            JSONObject result = new JSONObject();
            result.put("package", PACKAGE);
            result.put("versionName", versionName);
            result.put("versionCode", positiveCode(metadata, "SS_VERSION_CODE"));
            result.put("updateVersionCode", positiveCode(metadata, "UPDATE_VERSION_CODE"));
            result.put("manifestVersionCode", String.valueOf(Build.VERSION.SDK_INT >= 28
                ? info.getLongVersionCode() : info.versionCode));
            result.put("userAgent", System.getProperty("http.agent"));
            String defaultAgent = cronetDefaultUserAgent(context, application, info);
            if (defaultAgent != null) result.put("cronetDefaultUserAgent", defaultAgent);
            System.out.println(result.toString());
        } catch (Throwable error) {
            System.err.println("Cannot read installed Douyin package metadata: "
                + error.getClass().getSimpleName());
            System.exit(1);
        }
    }
}
