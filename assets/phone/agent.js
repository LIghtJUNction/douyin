// Run only inside the installed app. Never export its Cookies or signing keys.
const appExpected = { value: null };
const commonKeys = { value: null };
const readPaths = new Set(globalThis.douyinReadPaths);
function failure(code) { throw new Error(code); }
function context() {
  const users = Java.use('com.ss.android.ugc.aweme.account.AccountProxyService').userService();
  if (!users.isLogin()) failure('SESSION_MISSING');
  if (String(users.getCurUserId()) !== appExpected.value.userId) failure('ACCOUNT_CHANGED');
  const map = Java.use('java.util.HashMap').$new();
  Java.use('com.ss.android.common.applog.NetUtil').putCommonParams(map, true);
  const values = Object.create(null);
  const iterator = map.entrySet().iterator();
  while (iterator.hasNext()) {
    const row = Java.cast(iterator.next(), Java.use('java.util.Map$Entry'));
    values[String(row.getKey())] = String(row.getValue());
  }
  for (const [key, expected] of [['device_id', 'deviceId'], ['iid', 'installId']]) {
    if (values[key] !== appExpected.value[expected]) failure('DEVICE_CHANGED');
  }
  for (const [key, expected] of [['version_code', 'versionCode'], ['manifest_version_code', 'manifestVersionCode'], ['update_version_code', 'updateVersionCode']]) {
    if (values[key] !== appExpected.value[expected]) failure('APP_CHANGED');
  }
  if (values.aid !== '1128') failure('APP_CHANGED');
  commonKeys.value = new Set(Object.keys(values));
  return values;
}
function perform(callback) {
  return new Promise((resolve, reject) => Java.perform(() => {
    try { resolve(callback()); }
    catch (error) {
      const text = String(error);
      const code = ['SESSION_MISSING', 'ACCOUNT_CHANGED', 'DEVICE_CHANGED', 'APP_CHANGED', 'BAD_REQUEST', 'PHONE_GET_FAILED']
        .find(code => text.includes(code)) || 'APP_BRIDGE_UNSUPPORTED';
      reject(new Error(code));
    }
  }));
}
rpc.exports = {
  init(expected) {
    return perform(() => {
      // The obfuscated GET entrypoint is version-specific. Never guess its
      // meaning after an app update merely because a method name still exists.
      if (expected.versionCode !== '400800' || expected.manifestVersionCode !== '400801' || expected.updateVersionCode !== '40809900') {
        failure('APP_BRIDGE_UNSUPPORTED');
      }
      appExpected.value = Object.freeze(expected);
      context(); return { ready: true };
    });
  },
  fetch(path, params) {
    return perform(() => {
      if (!appExpected.value || typeof path !== 'string' || path.length > 4096 ||
          !/^\/(aweme|passport)\/[a-zA-Z0-9_./-]+\/?$/.test(path) ||
          path.split('/').some(segment => segment === '.' || segment === '..') ||
          !readPaths.has(path) || !Array.isArray(params) || params.length > 100) failure('BAD_REQUEST');
      const values = context();
      const seen = new Set();
      for (const row of params) {
        if (!Array.isArray(row) || row.length !== 2 || row.some(value => typeof value !== 'string' || value.length > 16384) ||
            commonKeys.value.has(row[0]) || seen.has(row[0])) failure('BAD_REQUEST');
        seen.add(row[0]); values[row[0]] = row[1];
      }
      const builder = Java.use('android.net.Uri').parse('https://aweme.snssdk.com' + path).buildUpon();
      for (const [key, value] of Object.entries(values)) builder.appendQueryParameter(key, value);
      // LJII is the direct GET provider. LJFF/LJI have legacy HTTP retry logic.
      const network = Java.use('com.ss.android.common.util.NetworkUtils');
      const body = network.LJII.overload('int', 'java.lang.String', 'java.util.List', 'boolean')
        .call(network, 16777216, String(builder.build()), null, true);
      if (body === null || String(body).length > 16777216) failure('PHONE_GET_FAILED');
      // Account switching while the request is in flight must not export data
      // under the previously imported account's identity.
      context();
      try { return JSON.parse(String(body)); } catch (_) { failure('PHONE_GET_FAILED'); }
    });
  }
};
