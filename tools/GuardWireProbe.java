import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.os.Looper;
import dalvik.system.PathClassLoader;
import java.io.StringWriter;
import java.io.Writer;

public final class GuardWireProbe {
 public static void main(String[] args) {
  try {
   Looper.prepareMainLooper();
   Class<?> at=Class.forName("android.app.ActivityThread");
   Object thread=at.getMethod("systemMain").invoke(null);
   Context context=(Context)at.getMethod("getSystemContext").invoke(thread);
   android.content.pm.PackageInfo installed=context.getPackageManager().getPackageInfo("com.ss.android.ugc.aweme",0);
   if (!"40.8.0".equals(installed.versionName) || installed.versionCode != 400801) throw new IllegalStateException("Unsupported fixture build");
   ApplicationInfo app=context.getPackageManager().getApplicationInfo("com.ss.android.ugc.aweme",0);
   ClassLoader loader=new PathClassLoader(app.sourceDir,app.nativeLibraryDir,GuardWireProbe.class.getClassLoader());
   if (args.length == 1 && "--proofs".equals(args[0])) {
    Class<?> proofType=loader.loadClass("com.bytedance.android.sdk.bdticketguard.TicketDataBean$CombineTsSign");
    // Check the installed model's actual Gson field mappings without starting
    // application services or initializing Keva in this temporary process.
    Class<?> serializedName=loader.loadClass("com.google.gson.annotations.SerializedName");
    for (String[] field:new String[][]{{"LIZ","ts_sign"},{"LIZIZ","ts_sign_ree"}}) {
     boolean mapped=false;
     for (java.lang.annotation.Annotation annotation:proofType.getField(field[0]).getAnnotations()) {
      if (annotation.annotationType().equals(serializedName)) mapped=field[1].equals(serializedName.getMethod("value").invoke(annotation));
     }
     if (!mapped) throw new IllegalStateException("Unexpected proof field mapping");
    }
    org.json.JSONArray cases=new org.json.JSONArray();
    String[] inputs={"fixture-legacy-proof","{\"ts_sign\":\"fixture-tee\",\"ts_sign_ree\":\"fixture-ree\"}","{\"ts_sign\":\"fixture-tee\"}","{\"ts_sign_ree\":\"fixture-ree\"}","{\"ts_sign\":null,\"ts_sign_ree\":null}","{}"};
    for (String input:inputs) {
     String teeInput=input,reeInput=null;
     if (input.startsWith("{")) {
      org.json.JSONObject object=new org.json.JSONObject(input);
      teeInput=object.isNull("ts_sign") ? null : object.getString("ts_sign");
      reeInput=object.isNull("ts_sign_ree") ? null : object.getString("ts_sign_ree");
     }
     Object proof=proofType.getConstructor(String.class,String.class).newInstance(teeInput,reeInput);
     Object tee=proofType.getMethod("LIZIZ").invoke(proof);
     Object ree=proofType.getMethod("LIZJ").invoke(proof);
     org.json.JSONObject item=new org.json.JSONObject();
     item.put("input",input);
     item.put("ts_sign",tee == null ? org.json.JSONObject.NULL : tee);
     item.put("ts_sign_ree",ree == null ? org.json.JSONObject.NULL : ree);
     cases.put(item);
    }
    System.out.println(cases.toString());
    return;
   }
   Class<?> model=loader.loadClass("X.1YtB");
   Object data=model.getConstructor(String.class,String.class,long.class).newInstance("ticket,path,timestamp","",1700000000L);
   model.getMethod("LJIIJJI",String.class).invoke(data,"fixture-proof");
   model.getMethod("LJIIIZ",String.class).invoke(data,"AQID");
   StringWriter output=new StringWriter();
   Class<?> writerType=loader.loadClass("com.google.gson.stream.JsonWriter");
   Object writer=writerType.getConstructor(Writer.class).newInstance(output);
   Class<?> adapterType=loader.loadClass("com.bytedance.android.sdk.bdticketguard.ClientDataTypeAdapter");
   Object adapter=adapterType.getConstructor().newInstance();
   adapterType.getMethod("write",writerType,Object.class).invoke(adapter,writer,data);
   System.out.println(output.toString());
  } catch(Throwable error) {
   System.err.println("Guard fixture probe failed: "+error.getClass().getSimpleName());
   for (Throwable cause=error; cause != null; cause=cause.getCause()) {
    System.err.println("Cause type: "+cause.getClass().getName());
    StackTraceElement[] frames=cause.getStackTrace();
    for (int i=0;i<Math.min(frames.length,3);i++) System.err.println(frames[i].toString());
   }
   System.exit(1);
  }
 }
}
