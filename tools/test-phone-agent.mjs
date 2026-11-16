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
  'java.net.URLEncoder': { encode: { overload: (...types) => {
    assert.deepEqual(types, ['java.lang.String', 'java.lang.String']);
    return { call: (_receiver, text, charset) => {
      assert.equal(charset, 'UTF-8');
      return new URLSearchParams([['value', text]]).toString().slice('value='.length);
    } };
  } } },
  'java.lang.String': { $new: value => ({ getBytes: { overload: type => {
    assert.equal(type, 'java.lang.String');
    return { call: (_receiver, charset) => { assert.equal(charset, 'UTF-8'); return Buffer.from(value, 'utf8'); } };
  } } }) },
  'X.18sM': { NONE: { value: 'NONE-fixture' } },
  'X.1Dpz': { NONE: { value: 'NONE-fixture' } },
  'android.net.Uri': { parse: raw => ({ buildUpon: () => {
    const url = new URL(raw);
    return { appendQueryParameter: (key, value) => url.searchParams.append(key, value), build: () => url.toString() };
  } }) },
  'com.ss.android.common.util.NetworkUtils': { LJII: (limit, url, headers, provider) => {
    assert.equal(limit, 16777216); assert.equal(headers, null); assert.equal(provider, true);
    requests.push(new URL(url)); afterNetwork(); return JSON.stringify({ status_code: 0, data: [{ aweme_id: '3' }] });
  }, LJIIIZ: (limit, url, bytes, compression, contentType, headers) => {
    assert.equal(limit, 16777216); assert.equal(compression, 'NONE-fixture'); assert.equal(headers, null);
    assert.equal(contentType, 'application/x-www-form-urlencoded; charset=UTF-8');
    const request = new URL(url);
    request.form = new URLSearchParams(Buffer.from(bytes).toString('utf8'));
    requests.push(request); afterNetwork();
    return JSON.stringify({ status_code: 0, data: { room: { id_str: '987' } } });
  } }
};
const sandbox = { douyinReadPaths:JSON.parse(readFileSync(new URL('../assets/phone/read-paths.json', import.meta.url),'utf8')),douyinLivePaths:JSON.parse(readFileSync(new URL('../assets/phone/live-paths.json', import.meta.url),'utf8')),rpc: { exports: {} }, Java: { perform: callback => callback(), cast: value => value, use: name => {
  assert.ok(name in classes, `unexpected class ${name}`); return classes[name];
} } };
classes['com.ss.android.common.util.NetworkUtils'].LJII.overload = (...types) => {
  assert.deepEqual(types, ['int', 'java.lang.String', 'java.util.List', 'boolean']);
  return { call: (_receiver, ...args) => classes['com.ss.android.common.util.NetworkUtils'].LJII(...args) };
};
classes['com.ss.android.common.util.NetworkUtils'].LJIIIZ.overload = (...types) => {
  assert.ok(['X.18sM', 'X.1Dpz'].includes(types[3]));
  assert.deepEqual(types, ['int', 'java.lang.String', '[B', types[3], 'java.lang.String', 'java.util.List']);
  return { call: (_receiver, ...args) => classes['com.ss.android.common.util.NetworkUtils'].LJIIIZ(...args) };
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
await agent.request('GET', 'https://webcast.amemv.com/webcast/room/info/', [['room_id', '987']], null);
assert.equal(requests.at(-1).origin, 'https://webcast.amemv.com');
assert.equal(requests.at(-1).searchParams.get('room_id'), '987');
assert.equal(requests.at(-1).searchParams.get('device_id'), '111');
await agent.request('GET', 'https://webcast.amemv.com/webcast/room/ping/anchor/', [['room_id', '987'], ['stream_id', '654'], ['status', '2']], null);
assert.equal(requests.at(-1).searchParams.get('stream_id'), '654');
assert.equal(requests.at(-1).searchParams.get('status'), '2');
await agent.request('POST', 'https://webcast.amemv.com/webcast/room/create/', [], [['title', 'fixture']]);
assert.equal(requests.at(-1).form.get('title'), 'fixture');
const current = { ...expected, versionCode: '410100', manifestVersionCode: '410101', updateVersionCode: '41109900' };
Object.assign(common, { version_code: current.versionCode, manifest_version_code: current.manifestVersionCode, update_version_code: current.updateVersionCode });
await agent.init(current);
const title = '直播 + & / % 中文';
await agent.request('POST', 'https://webcast.amemv.com/webcast/room/create/', [], [['title', title], ['gift_auth', '1']]);
assert.equal(requests.at(-1).origin, 'https://webcast.amemv.com');
assert.equal(requests.at(-1).form.get('title'), title);
assert.equal(requests.at(-1).form.get('gift_auth'), '1');
assert.equal(requests.at(-1).searchParams.has('title'), false, 'form values must stay in the body');
assert.equal(requests.at(-1).searchParams.get('iid'), '222');
for (const [endpoint, params, form] of [
  ['https://webcast.amemv.com/webcast/room/create/', [], [['iid', 'wrong']]],
  ['https://webcast.amemv.com/webcast/room/create/', [['title', 'query']], [['title', 'body']]],
  ['https://webcast.amemv.com/webcast/gift/send/', [], [['gift_id', '1']]],
  ['https://evil.invalid/webcast/room/create/', [], [['title', 'fixture']]],
  ['/aweme/v1/commit/item/digg/', [], []],
  ['https://webcast.amemv.com/webcast/room/info/', [], []]
]) {
  await assert.rejects(agent.request('POST', endpoint, params, form), /BAD_REQUEST/);
}
Object.assign(common, { version_code: expected.versionCode, manifest_version_code: expected.manifestVersionCode, update_version_code: expected.updateVersionCode });
await agent.init(expected);
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
for (const endpoint of [
  'https://webcast.amemv.com.evil.invalid/webcast/room/info/',
  'http://webcast.amemv.com/webcast/room/info/',
  'https://webcast.amemv.com:443/webcast/room/info/',
  'https://webcast.amemv.com@evil.invalid/webcast/room/info/',
  'https://webcast.amemv.com/webcast/room/info/?room_id=1',
  'https://webcast.amemv.com/webcast/room/info/#fragment',
  'https://webcast.amemv.com/webcast/room/../info/',
  'https://webcast.amemv.com/webcast/room/create/',
  'https://webcast.amemv.com/webcast/room/leave/',
  'https://webcast.amemv.com/webcast/gift/send/',
  '/webcast/room/info/'
]) {
  await assert.rejects(agent.request('GET', endpoint, [], null), /BAD_REQUEST/);
}
await assert.rejects(agent.request('GET', 'https://webcast.amemv.com/webcast/room/info/', [['iid', 'wrong']], null), /BAD_REQUEST/);
await assert.rejects(agent.request('GET', 'https://webcast.amemv.com/webcast/room/info/', [['room_id', '1'], ['room_id', '2']], null), /BAD_REQUEST/);
await assert.rejects(agent.request('GET', 'https://webcast.amemv.com/webcast/room/info/', [], []), /BAD_REQUEST/);
assert.equal(requests.length, before, 'invalid identities or requests must never reach the network provider');
afterNetwork = () => { active = 'changed-during-request'; };
await assert.rejects(agent.fetch('/aweme/v1/aweme/post/', []), /ACCOUNT_CHANGED/);
assert.equal(requests.length, before + 1, 'account switching during a request must discard its response');
console.log('phone agent identity, official livestream routes, form encoding and direct provider checks passed');
