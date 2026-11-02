import test from "node:test";
import assert from "node:assert/strict";
import { visibleProjects, visibleThreads, membershipLabel } from "../ui/view-model.mjs";

const catalog = {
  projects: [
    {
      project: { id: "a", name: "프로젝트 10", roots: [{ path: "D:/자료" }] },
      threads: [{ id: "t1", name: "회의" }],
    },
    {
      project: { id: "b", name: "프로젝트 2", roots: [{ path: "E:/작업" }] },
      threads: [{ id: "t2", name: "기획" }],
    },
  ],
  unassigned: [],
  unresolved: [],
};

test("sorts naturally and reverses without mutating the original order", () => {
  assert.deepEqual(
    visibleProjects(catalog, "", "asc").map((p) => p.project.id),
    ["b", "a"],
  );
  assert.deepEqual(
    visibleProjects(catalog, "", "desc").map((p) => p.project.id),
    ["a", "b"],
  );
  assert.equal(catalog.projects[0].project.id, "a");
});

test("search matches project name, multiple folder paths and conversation titles", () => {
  assert.deepEqual(
    visibleProjects(catalog, "자료", "asc").map((p) => p.project.id),
    ["a"],
  );
  assert.deepEqual(
    visibleProjects(catalog, "기획", "asc").map((p) => p.project.id),
    ["b"],
  );
  assert.deepEqual(visibleProjects(catalog, "없는 이름", "asc"), []);
});


test("normal counts include archived and no-user-event records; internal toggle preserves catalog", () => {
  const records = [
    {id:"normal",hasUserEvent:false,membershipReason:"folder"},
    {id:"archived",archived:true,hasUserEvent:false,membershipReason:"projectless"},
    {id:"internal",internal:true,membershipReason:"parent",name:"hidden needle"},
  ];
  const catalog = {projects:[{project:{id:"p",name:"Project",roots:[]},threads:records}]};
  assert.deepEqual(visibleThreads(records).map(t=>t.id),["normal","archived"]);
  assert.equal(visibleThreads(records,"normal-active").length,1);
  assert.equal(visibleThreads(records,"all").length,3);
  assert.equal(visibleThreads(records,"all-active").length,2);
  assert.equal(visibleProjects(catalog,"needle","asc").length,0);
  assert.equal(visibleProjects(catalog,"needle","asc","all").length,1);
  assert.equal(catalog.projects[0].threads.length,3,"backup/dependency selection retains internal data");
  assert.match(membershipLabel(records[0]),/표시만/);
  assert.match(membershipLabel(records[1]),/없음으로 설정/);
  assert.match(membershipLabel({membershipReason:"ambiguous"}),/확인 필요/);
});
