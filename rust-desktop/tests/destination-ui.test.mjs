import test from "node:test";
import assert from "node:assert/strict";
import { destinationBrowser } from "../ui/destination-ui.mjs";
class Element {
  constructor() { this.children=[]; this.events={}; this.value=""; }
  append(child) { this.children.push(child); }
  replaceChildren() { this.children=[]; }
  addEventListener(event, fn) { this.events[event]=fn; }
}
test("destination browser navigates chosen unregistered directories and ignores late navigation", async()=>{
  const elements=new Map(), calls=[]; let resolveOld;
  const el=(id)=>{if(!elements.has(id))elements.set(id,new Element());return elements.get(id);};
  const browser=destinationBrowser({element:el,document:{createElement:()=>new Element()},invoke:async()=>"E:/chosen",error:assert.fail,transfer:async()=>{},job:async(action,args)=>{
    calls.push({action,args});
    if(action==="pane_preferences")return {};
    if(args.path==="old")return new Promise(resolve=>{resolveOld=resolve;});
    return {path:args.path,parent:"E:/",folders:[{name:"한글".repeat(100),path:"E:/chosen/long"},{name:"Other",path:"E:/chosen/other"}]};
  }});
  const old=browser.navigate("old"); await browser.navigate("E:/chosen");
  resolveOld({path:"old",parent:"",folders:[]}); await old;
  assert.equal(browser.path(),"E:/chosen");assert.equal(el("destination-list").children.length,2);
  el("destination-search").value="한글";el("destination-search").events.input();
  assert.equal(el("destination-list").children.length,1);
  assert.equal(calls.filter(c=>c.action==="pane_preferences").length,1);
  assert.equal(calls.some(c=>["catalog","portable_restore"].includes(c.action)),false);
});
