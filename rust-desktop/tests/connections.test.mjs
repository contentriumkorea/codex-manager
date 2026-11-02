import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { membershipLabel } from "../ui/view-model.mjs";
const source = readFileSync(new URL("../ui/main.mjs", import.meta.url), "utf8");
const start = source.indexOf("async function runJob(");
const end = source.indexOf("async function newProject()", start);
function harness(answer, change = false, fail = false, staleEmpty = false) {
  const calls = [], text = [];
  const state = {home:"A",catalog:{revision:"original"}};
  const plan = {projects:["p"],revision:"original",groups:[{project:{id:"p",name:"Project"},threads:[{id:"t",name:"Normal",membershipReason:"folder"},{id:"c",internal:true,membershipReason:"parent"}]}]};
  const context = {
    state, membershipLabel, titleOf:t=>t.name||t.id,
    invoke:async (command,args)=>{calls.push([command,args]);return "job";},
    pollJob:async ()=>{if(staleEmpty){state.home="other-store";return {...plan,groups:[]};}return plan;},
    dialog:async (_title,build)=>{build({append:(...items)=>text.push(...items)});if(change)state.catalog.revision="changed";return answer;},
    node:(_tag,value)=>value,
    render:()=>{},setStatus:()=>{},
    bridge:{runVisible:async(action,args)=>{calls.push([action,args]);if(fail)throw Error("connection interrupted");if(action==="confirm_connections")state.catalog.revision="connected";return {};}}
  };
  vm.runInNewContext(source.slice(start,end),context);
  return {calls,state,text,run:()=>context.runJob("transfer",{id:"p",destinations:["B"]})};
}
test("connection preview cancellation never sends a write or starts the requested operation",async()=>{
  const h=harness(false);assert.equal(await h.run(),null);
  assert.deepEqual(h.calls.map(c=>c[0]),["start_job"]);
  assert.equal(h.calls[0][1].action,"connection_plan");
  assert.match(h.text.join("\n"),/Normal · t/);assert.match(h.text.join("\n"),/c · c/);
});
test("confirmed connection precedes project mutation with the refreshed revision",async()=>{
  const h=harness(true);await h.run();
  assert.deepEqual(h.calls.map(c=>c[0]),["start_job","confirm_connections","transfer"]);
  assert.equal(h.calls[1][1].expectedRevision,"original");
  assert.equal(h.calls[2][1].expectedRevision,"connected");
});
test("a stale preview or interrupted connection never proceeds to project mutation",async()=>{
  const stale=harness(true,true);await assert.rejects(stale.run(),/바뀌었습니다/);
  assert.deepEqual(stale.calls.map(c=>c[0]),["start_job"]);
  const interrupted=harness(true,false,true);await assert.rejects(interrupted.run(),/interrupted/);
  assert.deepEqual(interrupted.calls.map(c=>c[0]),["start_job","confirm_connections"]);
  assert.equal(interrupted.state.busy,false);
});

test("store switch during an empty connection preview cannot reroute the project mutation",async()=>{
  const h=harness(true,false,false,true);await assert.rejects(h.run(),/바뀌었습니다/);
  assert.deepEqual(h.calls.map(c=>c[0]),["start_job"]);
});
