import dalvik.system.PathClassLoader;
import java.io.FileInputStream;
import java.security.MessageDigest;
import java.util.HashMap;
import java.util.Map;
import org.json.JSONArray;
import org.json.JSONObject;

/** Exercises only u2's Java serializer with public fake values, never SDK init. */
public final class MetasecWireProbe {
    private static final String[] TEXT_FIELDS = {
        "LIZ", "LJII", "LJI", "LJIIIIZZ", "LIZIZ", "LIZJ", "LIZLLL", "LJ", "LJFF"
    };

    private static JSONArray entries(Map<String, Object> map) throws Exception {
        JSONArray result = new JSONArray();
        for (Map.Entry<String, Object> entry : map.entrySet()) {
            JSONArray pair = new JSONArray();
            pair.put(entry.getKey() == null ? JSONObject.NULL : entry.getKey());
            pair.put(entry.getValue() == null ? JSONObject.NULL : entry.getValue());
            result.put(pair);
        }
        return result;
    }

    private static java.lang.reflect.Field field(Class<?> type, String name) throws Exception {
        java.lang.reflect.Field field = type.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    public static void main(String[] args) {
        try {
            if (args.length != 1) throw new IllegalArgumentException("APK path required");
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            try (FileInputStream input = new FileInputStream(args[0])) {
                byte[] buffer = new byte[32768];
                int count;
                while ((count = input.read(buffer)) != -1) digest.update(buffer, 0, count);
            }
            StringBuilder hash = new StringBuilder();
            for (byte value : digest.digest()) hash.append(String.format("%02x", value & 255));
            if (!hash.toString().equals("a4e0682ff174d73dbc00d77622037090ee88f164fc0f9b62c324a1234e64ba57"))
                throw new IllegalArgumentException("Unsupported fixture APK");
            ClassLoader parent = MetasecWireProbe.class.getClassLoader();
            ClassLoader loader = new PathClassLoader(args[0], parent);
            // Only the version holder is replaced. The serializer itself must
            // come from the APK, with no native library or application startup.
            if (loader.loadClass("ms.bd.c.s4") != ms.bd.c.s4.class)
                throw new IllegalStateException("Fixture version holder not selected");
            Class<?> type = loader.loadClass("ms.bd.c.u2");
            if (type.getClassLoader() != loader)
                throw new IllegalStateException("Serializer did not come from APK");
            // u2 is abstract. Its concrete wrapper only calls the base
            // constructor and stores this unused reference; no SDK init runs.
            java.lang.reflect.Constructor<?> constructor = loader.loadClass("X.0mVm")
                .getConstructor(loader.loadClass("X.0mVu"));
            constructor.setAccessible(true);
            java.lang.reflect.Method serializer = type.getDeclaredMethod("LIZ");
            serializer.setAccessible(true);
            JSONArray cases = new JSONArray();
            Object defaults = constructor.newInstance(new Object[] {null});
            JSONObject first = new JSONObject();
            first.put("case", "defaults");
            first.put("wire", new JSONArray((String) serializer.invoke(defaults)));
            cases.put(first);

            Object configured = constructor.newInstance(new Object[] {null});
            String[] values = {
                "\t fixture-app \r", " opaque-1 ", " secondary ", " fixture/license ",
                null, "\u00a0device-fixture\u00a0", " opaque-2 ", " install-fixture ", " opaque-3 "
            };
            for (int i = 0; i < TEXT_FIELDS.length; i++)
                field(type, TEXT_FIELDS[i]).set(configured, values[i]);
            field(type, "LJIIJ").setInt(configured, Integer.MIN_VALUE);
            field(type, "LJIIJJI").setInt(configured, Integer.MAX_VALUE);
            field(type, "LJIIL").setInt(configured, -7);
            field(type, "LJIJI").set(configured, " fallback-device ");
            field(type, "LJIJJ").setInt(configured, 1);
            HashMap<String, Object> parameters = new HashMap<String, Object>();
            parameters.put(null, "ignored-null-key");
            parameters.put("", "ignored-empty-key");
            parameters.put(" \t ", Integer.valueOf(7));
            parameters.put(" text ", " quote\" / slash\\ and\nnewline ");
            parameters.put("nullable", null);
            parameters.put("unicode", "\u00a0 kept \u00a0");
            StringBuilder escaping = new StringBuilder("prefix");
            for (int i = 0; i < 32; i++) escaping.append((char) i);
            escaping.append("</script>\u2028\u2029");
            parameters.put("escaping", escaping.toString());
            HashMap<String, Object> options = new HashMap<String, Object>();
            options.put("tk_key", " fixture ");
            options.put("boolean", Boolean.TRUE);
            field(type, "LJIILIIL").set(configured, parameters);
            field(type, "LJIILJJIL").set(configured, options);
            JSONObject second = new JSONObject();
            second.put("case", "configured");
            second.put("parameters", entries(parameters));
            second.put("options", entries(options));
            String wire = (String) serializer.invoke(configured);
            second.put("wire", new JSONArray(wire));
            second.put("wireText", wire);
            cases.put(second);
            System.out.println(cases.toString());
        } catch (Throwable error) {
            System.err.println("Metasec fixture failed: " + error.getClass().getSimpleName());
            System.exit(1);
        }
    }
}
