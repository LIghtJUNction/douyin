// Run only inside the installed app. Never export its Cookies or signing keys.
const appExpected = { value: null };
const bridgeProfile = { value: null };
const commonKeys = { value: null };
const readPaths = new Set(globalThis.douyinReadPaths);
// Both read requests and explicit anchor state changes use these fixed GET
// paths. Confirmation for state changes belongs to the CLI command boundary.
const livePaths = Object.fromEntries(Object.entries(globalThis.douyinLivePaths).map(([method, paths]) => [method, new Set(paths)]));
const liveOrigin = 'https://webcast.amemv.com';
const profiles = [
  { version: ['400800', '400801', '40809900'], postCompression: 'X.1Dpz' },
  { version: ['410100', '410101', '41109900'], postCompression: 'X.18sM' }
];
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
      const profile = profiles.find(profile => profile.version.every((version, index) =>
        version === expected[['versionCode', 'manifestVersionCode', 'updateVersionCode'][index]]));
      if (!profile) failure('APP_BRIDGE_UNSUPPORTED');
      bridgeProfile.value = profile;
      appExpected.value = Object.freeze(expected);
      context(); return { ready: true };
    });
  },
  fetch(path, params) {
    return rpc.exports.request('GET', path, params, null);
  },
  request(method, endpoint, params, form) {
    return perform(() => {
      if (!appExpected.value || !['GET', 'POST'].includes(method) ||
          typeof endpoint !== 'string' || endpoint.length > 4096) failure('BAD_REQUEST');
      const isLive = endpoint.startsWith(liveOrigin + '/');
      const path = isLive ? endpoint.slice(liveOrigin.length) : endpoint;
      if (isLive ? !livePaths[method].has(path) : (
          method !== 'GET' ||
          !/^\/(aweme|passport)\/[a-zA-Z0-9_./-]+\/?$/.test(path) ||
          path.split('/').some(segment => segment === '.' || segment === '..') ||
          !readPaths.has(path))) failure('BAD_REQUEST');
      if (!Array.isArray(params) || params.length > 100 ||
          (method === 'GET' ? form != null : !Array.isArray(form) || form.length > 100)) failure('BAD_REQUEST');
      if (method === 'POST' && !bridgeProfile.value.postCompression) failure('APP_BRIDGE_UNSUPPORTED');
      const values = context();
      const seen = new Set();
      for (const row of [...params, ...(form || [])]) {
        if (!Array.isArray(row) || row.length !== 2 || row.some(value => typeof value !== 'string' || value.length > 16384) ||
            commonKeys.value.has(row[0]) || seen.has(row[0])) failure('BAD_REQUEST');
        seen.add(row[0]);
      }
      for (const [key, value] of params) values[key] = value;
      const builder = Java.use('android.net.Uri').parse((isLive ? liveOrigin : 'https://aweme.snssdk.com') + path).buildUpon();
      for (const [key, value] of Object.entries(values)) builder.appendQueryParameter(key, value);
      // LJII is the direct GET provider. LJFF/LJI have legacy HTTP retry logic.
      const network = Java.use('com.ss.android.common.util.NetworkUtils');
      let body;
      if (method === 'GET') {
        body = network.LJII.overload('int', 'java.lang.String', 'java.util.List', 'boolean')
          .call(network, 16777216, String(builder.build()), null, true);
      } else {
        // The verified direct POST providers accept exact form bytes and NONE
        // compression. Its legacy retry wrapper is deliberately not called.
        const encoder = Java.use('java.net.URLEncoder');
        const encode = value => String(encoder.encode.overload('java.lang.String', 'java.lang.String')
          .call(encoder, value, 'UTF-8'));
        const encoded = form.map(([key, value]) => encode(key) + '=' + encode(value)).join('&');
        const text = Java.use('java.lang.String').$new(encoded);
        const bytes = text.getBytes.overload('java.lang.String').call(text, 'UTF-8');
        const compression = bridgeProfile.value.postCompression;
        body = network.LJIIIZ.overload('int', 'java.lang.String', '[B', compression, 'java.lang.String', 'java.util.List')
          .call(network, 16777216, String(builder.build()), bytes, Java.use(compression).NONE.value,
            'application/x-www-form-urlencoded; charset=UTF-8', null);
      }
      if (body === null || String(body).length > 16777216) failure('PHONE_GET_FAILED');
      // Account switching while the request is in flight must not export data
      // under the previously imported account's identity.
      context();
      try { return JSON.parse(String(body)); } catch (_) { failure('PHONE_GET_FAILED'); }
    });
  }
};
