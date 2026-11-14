import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const expected = { userId: 'fixture-user', deviceId: '111', installId: '222', versionCode: '400800', manifestVersionCode: '400801', updateVersionCode: '40809900' };
const common = { aid: '1128', device_id: '111', iid: '222', version_code: '400800', manifest_version_code: '400801', update_version_code: '40809900' };
let active = 'fixture-user';
let loggedIn = true;
const requests = [];
let afterNetwork = () => {};
const classes = {
  'com.ss.android.ugc.aweme.account.AccountProxyService': { userService: () => ({ isLogin: () => loggedIn, getCurUserId: () => active }) },
  'java.util.HashMap': { $new: () => ({ entrySet: () => ({ iterator: () => {
    const rows = Object.entries(common); let index = 0;
    return { hasNext: () => index < rows.length, next: () => {
      const [key, value] = rows[index++]; return { getKey: () => key, getValue: () => value };
    } };
  } }) }) },
  'com.ss.android.common.applog.NetUtil': { putCommonParams: () => {} },
  'java.util.Map$Entry': {},
  'android.net.Uri': { parse: raw => ({ buildUpon: () => {
    const url = new URL(raw);
    return { appendQueryParameter: (key, value) => url.searchParams.append(key, value), build: () => url.toString() };
  } }) },
  'com.ss.android.common.util.NetworkUtils': { LJII: (limit, url, headers, provider) => {
    assert.equal(limit, 16777216); assert.equal(headers, null); assert.equal(provider, true);
    requests.push(new URL(url)); afterNetwork(); return JSON.stringify({ status_code: 0, data: [{ aweme_id: '3' }] });
  } }
};
const sandbox = { douyinReadPaths:JSON.parse(readFileSync(new URL('../assets/phone/read-paths.json', import.meta.url),'utf8')),rpc: { exports: {} }, Java: { perform: callback => callback(), cast: value => value, use: name => {
  assert.ok(name in classes, `unexpected class ${name}`); return classes[name];
} } };
classes['com.ss.android.common.util.NetworkUtils'].LJII.overload = (...types) => {
  assert.deepEqual(types, ['int', 'java.lang.String', 'java.util.List', 'boolean']);
  return { call: (_receiver, ...args) => classes['com.ss.android.common.util.NetworkUtils'].LJII(...args) };
};
vm.runInNewContext(readFileSync(new URL('../assets/phone/agent.js', import.meta.url), 'utf8'), sandbox);
const agent = sandbox.rpc.exports;
await assert.rejects(agent.init({ ...expected, versionCode: '400900' }), /APP_BRIDGE_UNSUPPORTED/);
await agent.init(expected);
const result = await agent.fetch('/aweme/v1/aweme/post/', [['sec_user_id', 'another-user'], ['count', '3']]);
assert.equal(result.data.length, 1);
assert.equal(requests[0].origin, 'https://aweme.snssdk.com');
assert.equal(requests[0].searchParams.get('device_id'), '111');
assert.equal(requests[0].searchParams.get('sec_user_id'), 'another-user');
const before = requests.length;
active = 'changed-user';
await assert.rejects(agent.fetch('/aweme/v1/aweme/post/', []), /ACCOUNT_CHANGED/);
active = 'fixture-user'; loggedIn = false;
await assert.rejects(agent.fetch('/aweme/v1/aweme/post/', []), /SESSION_MISSING/);
loggedIn = true; common.iid = '333';
await assert.rejects(agent.fetch('/aweme/v1/aweme/post/', []), /DEVICE_CHANGED/);
common.iid = '222'; common.version_code = '400900';
await assert.rejects(agent.fetch('/aweme/v1/aweme/post/', []), /APP_CHANGED/);
common.version_code = '400800';
for (const path of ['https://example.invalid/aweme/', '//example.invalid/aweme/', '/aweme/../passport/', '/aweme/x?redirect=other', '/aweme/%2e%2e/', '/aweme/v1/commit/item/digg/', '/passport/logout/']) {
  await assert.rejects(agent.fetch(path, []), /BAD_REQUEST/);
}
for (const params of [[['device_id', 'wrong']], [['count', '1'], ['count', '2']]]) {
  await assert.rejects(agent.fetch('/aweme/v1/aweme/post/', params), /BAD_REQUEST/);
}
assert.equal(requests.length, before, 'invalid identities or requests must never reach the network provider');
afterNetwork = () => { active = 'changed-during-request'; };
await assert.rejects(agent.fetch('/aweme/v1/aweme/post/', []), /ACCOUNT_CHANGED/);
assert.equal(requests.length, before + 1, 'account switching during a request must discard its response');
console.log('phone agent identity, origin, parameter and GET-provider checks passed');
