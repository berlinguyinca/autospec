#!/usr/bin/env bats
# tests/smoke/test_fleet_gui_html.bats — smoke tests for the fleet GUI
# frontend (skills/autospec-fleet/gui/index.html).
#
# Section A (grep): structural/attribute presence — fast, no runtime.
# Section B (node): real DOM/handler assertions executed via a minimal vm
#   sandbox with stub fetch/document (#841).  Requires node >= 16.
#   No jsdom or heavy dependencies — uses node's built-in `vm` module.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    GUI_HTML="$REPO_ROOT/skills/autospec-fleet/gui/index.html"
}

@test "fleet-gui index.html exists" {
    [ -f "$GUI_HTML" ]
}

@test "fleet-gui search box has aria-label='Filter repos by name'" {
    grep -q 'aria-label="Filter repos by name"' "$GUI_HTML"
}

@test "fleet-gui repo rows use label for='repo-N' pattern (dynamic setAttribute)" {
    grep -q "setAttribute.*'for'" "$GUI_HTML"
}

@test "fleet-gui Save button has Cmd/Ctrl+S keydown handler with preventDefault" {
    grep -q 'metaKey\|ctrlKey' "$GUI_HTML"
    grep -q "key === 's'" "$GUI_HTML"
    grep -q 'preventDefault' "$GUI_HTML"
}

@test "fleet-gui POSTs to /api/config with X-Autospec-Token header" {
    grep -q "X-Autospec-Token" "$GUI_HTML"
    grep -q '/api/config' "$GUI_HTML"
}

@test "fleet-gui does not persist the launch token in browser storage" {
    ! grep -q 'localStorage' "$GUI_HTML"
    grep -q 'history.replaceState' "$GUI_HTML"
}

@test "fleet-gui has Select all visible and Clear all controls" {
    grep -q 'select-all' "$GUI_HTML"
    grep -q 'clear-all' "$GUI_HTML"
}

@test "fleet-gui two-column grid layout (35%/65%)" {
    grep -q '35% 65%' "$GUI_HTML"
}

# ---------------------------------------------------------------------------
# Section B — real DOM/handler assertions via node vm sandbox (#841)
#
# Executes the inline <script> from index.html inside a node vm context with
# a minimal stub document/fetch (no jsdom required).  Asserts handler wiring
# that grep-on-source cannot detect: checkbox change→enabledUrls, save()→POST.
# ---------------------------------------------------------------------------

# skip_if_no_node: mark a test as skipped when node is absent.
skip_if_no_node() {
    if ! command -v node >/dev/null 2>&1; then
        skip "node not available — Section B DOM tests skipped"
    fi
}

# run_dom_test <node-script-content>: write to a temp file and run it.
# The test receives GUI_HTML as the first argument.
run_dom_test() {
    local script="$1"
    local tmpjs
    tmpjs="$(mktemp /tmp/fleet-dom-test.XXXXXX.js)"
    printf '%s\n' "$script" > "$tmpjs"
    node "$tmpjs" "$GUI_HTML"
    local rc=$?
    rm -f "$tmpjs"
    return "$rc"
}

@test "fleet-gui DOM: checkbox change handler updates enabledUrls (#841)" {
    skip_if_no_node
    run run_dom_test '
const fs = require("fs"); const vm = require("vm"); const { URLSearchParams } = require("url");
const html = fs.readFileSync(process.argv[2], "utf8");
const m = html.match(/<script>([\s\S]*?)<\/script>/);
if (!m) { console.error("no inline script"); process.exit(1); }

// Minimal DOM factory
function makeEl() {
  const l = {};
  const el = { _ch:[], _at:{}, dataset:{}, className:"", id:"", textContent:"", innerHTML:"",
    type:"", checked:false, value:"", style:{},
    addEventListener(t,f){ (l[t]||(l[t]=[])).push(f); },
    dispatchEvent(t,e){ (l[t]||[]).forEach(f=>f(e||{})); },
    setAttribute(k,v){ this._at[k]=v; }, getAttribute(k){ return this._at[k]??null; },
    appendChild(c){ this._ch.push(c); return c; },
    querySelectorAll(sel){ const r=[]; (function w(n){ n._ch&&n._ch.forEach(w);
      if(sel.includes("checkbox")&&n.type==="checkbox") r.push(n); })(root); return r; }
  };
  return el;
}
const root = makeEl();
const els = {};
["error-banner","banner","save-btn","workspace-path","cfg-workspace","cfg-profile",
 "cfg-parallel","filter-box","select-all","clear-all","repo-list"].forEach(id=>{
  const e=makeEl(); e.id=id; e.value=(id==="cfg-parallel")?"2":""; els[id]=e; root._ch.push(e);
});
const document = { createElement:()=>makeEl(), getElementById:(id)=>els[id]||makeEl(),
  addEventListener:(t,f)=>root.addEventListener(t,f), dispatchEvent:(t,e)=>root.dispatchEvent(t,e) };
const fetchCalls = [];
async function fetch(url,opts){ fetchCalls.push({url,opts});
  if(url==="/api/repos") return {ok:true, json:async()=>[
    {nameWithOwner:"org/a",url:"https://github.com/org/a",visibility:"PUBLIC",pushedAt:"2026-06-01T10:00:00Z",description:""},
    {nameWithOwner:"org/b",url:"https://github.com/org/b",visibility:"PUBLIC",pushedAt:"2026-05-30T12:00:00Z",description:""}
  ]};
  if(url==="/api/config") return {ok:true, json:async()=>({config:{workspace:"/w",default_profile:"",parallel_repos:2,
    repos:[{url:"https://github.com/org/a",enabled:true}]}})};
  return {ok:true, json:async()=>({})};
}
const ctx={document,window:{close(){}},location:{search:"?t=tok"},
  localStorage:{_s:{},getItem(k){return this._s[k]||null;},setItem(k,v){this._s[k]=v;}},
  fetch, URLSearchParams, Promise, setTimeout, clearTimeout, console};
vm.createContext(ctx); vm.runInContext(m[1], ctx);

(async()=>{
  await new Promise(r=>setTimeout(r,300));
  const cbs = els["repo-list"].querySelectorAll("checkbox");
  if(cbs.length!==2){console.error("FAIL: expected 2 checkboxes, got",cbs.length);process.exit(1);}
  const cbA=cbs.find(c=>c.dataset.url==="https://github.com/org/a");
  const cbB=cbs.find(c=>c.dataset.url==="https://github.com/org/b");
  if(!cbA||!cbB){console.error("FAIL: could not find checkboxes by url");process.exit(1);}
  if(!cbA.checked){console.error("FAIL: org/a should start checked");process.exit(1);}
  if(cbB.checked){console.error("FAIL: org/b should start unchecked");process.exit(1);}
  // Toggle org/b on
  cbB.checked=true; cbB.dispatchEvent("change",{});
  // Toggle org/a off
  cbA.checked=false; cbA.dispatchEvent("change",{});
  // Trigger save
  els["save-btn"].dispatchEvent("click",{});
  await new Promise(r=>setTimeout(r,200));
  const post=fetchCalls.find(c=>c.url==="/api/config"&&c.opts&&c.opts.method==="POST");
  if(!post){console.error("FAIL: no POST to /api/config");process.exit(1);}
  const body=JSON.parse(post.opts.body);
  const urls=body.repos.map(r=>r.url);
  if(urls.includes("https://github.com/org/a")){console.error("FAIL: org/a should be removed",urls);process.exit(1);}
  if(!urls.includes("https://github.com/org/b")){console.error("FAIL: org/b missing after toggle",urls);process.exit(1);}
  console.log("PASS");
})().catch(e=>{console.error(e);process.exit(1);});
'
    [ "$status" -eq 0 ]
    [[ "$output" == *"PASS"* ]]
}

@test "fleet-gui DOM: save() POSTs correct JSON and preserves extraConfig (#841)" {
    skip_if_no_node
    run run_dom_test '
const fs = require("fs"); const vm = require("vm"); const { URLSearchParams } = require("url");
const html = fs.readFileSync(process.argv[2], "utf8");
const m = html.match(/<script>([\s\S]*?)<\/script>/);
if (!m) { console.error("no inline script"); process.exit(1); }

function makeEl() {
  const l = {};
  return { _ch:[], _at:{}, dataset:{}, className:"", id:"", textContent:"", innerHTML:"",
    type:"", checked:false, value:"", style:{},
    addEventListener(t,f){ (l[t]||(l[t]=[])).push(f); },
    dispatchEvent(t,e){ (l[t]||[]).forEach(f=>f(e||{})); },
    setAttribute(k,v){ this._at[k]=v; }, getAttribute(k){ return this._at[k]??null; },
    appendChild(c){ this._ch.push(c); return c; },
    querySelectorAll(sel){ const r=[]; (function w(n){ n._ch&&n._ch.forEach(w);
      if(sel.includes("checkbox")&&n.type==="checkbox") r.push(n); })(root); return r; }
  };
}
const root = makeEl();
const els = {};
["error-banner","banner","save-btn","workspace-path","cfg-workspace","cfg-profile",
 "cfg-parallel","filter-box","select-all","clear-all","repo-list"].forEach(id=>{
  const e=makeEl(); e.id=id; e.value=(id==="cfg-parallel")?"2":""; els[id]=e; root._ch.push(e);
});
const document = { createElement:()=>makeEl(), getElementById:(id)=>els[id]||makeEl(),
  addEventListener:(t,f)=>root.addEventListener(t,f), dispatchEvent:(t,e)=>root.dispatchEvent(t,e) };
const fetchCalls=[];
async function fetch(url,opts){ fetchCalls.push({url,opts});
  if(url==="/api/repos") return {ok:true, json:async()=>[
    {nameWithOwner:"org/r1",url:"https://github.com/org/r1",visibility:"PUBLIC",pushedAt:"2026-06-01T10:00:00Z",description:""}
  ]};
  if(url==="/api/config") return {ok:true, json:async()=>({config:{workspace:"/myws",default_profile:"main",
    parallel_repos:4, repos:[{url:"https://github.com/org/r1",enabled:true}], extraKey:"keep-me"}})};
  return {ok:true, json:async()=>({})};
}
const ctx={document,window:{close(){}},location:{search:"?t=tok"},
  localStorage:{_s:{},getItem(k){return this._s[k]||null;},setItem(k,v){this._s[k]=v;}},
  fetch, URLSearchParams, Promise, setTimeout, clearTimeout, console};
vm.createContext(ctx); vm.runInContext(m[1], ctx);

(async()=>{
  await new Promise(r=>setTimeout(r,300));
  els["save-btn"].dispatchEvent("click",{});
  await new Promise(r=>setTimeout(r,200));
  const post=fetchCalls.find(c=>c.url==="/api/config"&&c.opts&&c.opts.method==="POST");
  if(!post){console.error("FAIL: no POST to /api/config; calls:", JSON.stringify(fetchCalls.map(c=>({url:c.url,method:c.opts?.method}))));process.exit(1);}
  const body=JSON.parse(post.opts.body);
  if(body.workspace!=="/myws"){console.error("FAIL: workspace wrong:",body.workspace);process.exit(1);}
  if(body.default_profile!=="main"){console.error("FAIL: default_profile wrong:",body.default_profile);process.exit(1);}
  if(body.parallel_repos!==4){console.error("FAIL: parallel_repos wrong:",body.parallel_repos);process.exit(1);}
  if(!Array.isArray(body.repos)||body.repos[0].url!=="https://github.com/org/r1"){
    console.error("FAIL: repos wrong:",body.repos);process.exit(1);}
  if(body.extraKey!=="keep-me"){console.error("FAIL: extraConfig not preserved:",JSON.stringify(body));process.exit(1);}
  const hdrs=post.opts.headers||{};
  const hasToken=Object.entries(hdrs).some(([k,v])=>k==="X-Autospec-Token"&&v==="tok");
  if(!hasToken){console.error("FAIL: X-Autospec-Token not in POST headers:",hdrs);process.exit(1);}
  console.log("PASS");
})().catch(e=>{console.error(e);process.exit(1);});
'
    [ "$status" -eq 0 ]
    [[ "$output" == *"PASS"* ]]
}
