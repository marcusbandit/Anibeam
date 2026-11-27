// Drives a running dev Worker over HTTP and checks it behaves as its README says.
// Node 22 or later (built-in fetch), no packages.
//
//   SMOKE_DEV_TOKEN=<token> node smoke.mjs <base-url>
//
// It checks that a usage log send is stored once per send id, that a bad or
// oversized one is refused, that every route but POST /usage needs the token
// and an unknown path is 404, that ?since= reads only newer sends, that
// DELETE ?before= prunes, and that the 6th send in a minute gets 429. It leaves
// exactly one send behind (calls.Scan 4) for smoke.sh's script checks.

const base = (process.argv[2] ?? "http://127.0.0.1:8796").replace(/\/$/, "");
const token = process.env.SMOKE_DEV_TOKEN ?? "";

let failures = 0;
function check(ok, what) {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
}

const usageLog = (scans) => ({
  format: 1,
  app_version: "0.0.0-smoke",
  platform: "desktop",
  os: "linux",
  days: { "2026-11-27": { calls: { Scan: scans }, pages: { library: 1 }, errors: {} } },
});

function hexId() {
  return [...crypto.getRandomValues(new Uint8Array(16))].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function call(method, path, { auth, body } = {}) {
  const headers = {};
  if (auth !== undefined) headers.authorization = `Bearer ${auth}`;
  if (body !== undefined) headers["content-type"] = "application/json";
  const res = await fetch(`${base}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body),
  });
  const text = await res.text();
  let json = null;
  try {
    json = JSON.parse(text);
  } catch {}
  return { status: res.status, text, json };
}

const send = (body) => call("POST", "/usage", { body }).then((r) => r.status);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

// Sends: five a minute per address, so the order here keeps two good ones in.
const first = hexId();
const second = hexId();
check((await send({ send_id: first, usage: usageLog(2) })) === 201, "a usage log send is stored");
check((await send({ send_id: first, usage: usageLog(2) })) === 200, "the same send id twice is one send");
await wait(20);
check((await send({ send_id: second, usage: usageLog(4) })) === 201, "a second send is stored");
check((await send("not json")) === 400, "a body that is not JSON is refused");
const big = { send_id: hexId(), usage: { ...usageLog(1), days: { pad: "x".repeat(300 * 1024) } } };
check((await send(big)) === 413, "a send over 256 KiB is refused");

// The owner's routes and the gate in front of them
check((await call("GET", "/usage")).status === 401, "reading without the token is refused");
check((await call("GET", "/usage", { auth: "wrong" })).status === 401, "reading with the wrong token is refused");
check((await call("DELETE", "/usage?before=9999999999999")).status === 401, "pruning without the token is refused");
check((await call("PUT", "/usage", { auth: token })).status === 405, "another method on /usage is 405");
check((await call("GET", "/nothing", { auth: token })).status === 404, "an unknown path is 404 with the token");
check((await call("POST", "/nothing")).status === 404, "an unknown path is 404 without it");
check((await call("GET", "/")).status === 404, "there is no page at /");
check((await call("GET", "/usage?since=soon", { auth: token })).status === 400, "a since that is not a number is 400");
check((await call("DELETE", "/usage", { auth: token })).status === 400, "a prune without before is 400");

const all = await call("GET", "/usage", { auth: token });
const sends = all.json?.sends ?? [];
const a = sends.find((s) => s.send_id === first);
const b = sends.find((s) => s.send_id === second);
check(all.status === 200 && sends.length === 2 && a && b, `the owner reads both sends back (got ${all.status}, ${sends.length})`);
check(a && a.usage.days["2026-11-27"].calls.Scan === 2, "with their counts");
check(all.json && all.json.next === null, "and next is null at the end");

if (a && b) {
  const newer = await call("GET", `/usage?since=${a.at}`, { auth: token });
  const ids = (newer.json?.sends ?? []).map((s) => s.send_id);
  check(ids.length === 1 && ids[0] === second, `since=<first's time> reads only the second (${ids.length})`);

  const pruned = await call("DELETE", `/usage?before=${b.at}`, { auth: token });
  check(pruned.status === 200 && pruned.json?.deleted === 1, `before=<second's time> prunes the first (${pruned.text})`);
  const left = await call("GET", "/usage", { auth: token });
  check(left.json?.sends?.length === 1 && left.json.sends[0].send_id === second, "the second is still there");
}

const statuses = [];
for (let i = 0; i < 6; i++) statuses.push(await send({ send_id: hexId(), usage: usageLog(1) }));
check(statuses.every((s) => s === 429), `sends past 5 a minute get 429 (${statuses.join(" ")})`);

if (failures) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
