import { getBuiltinProviders, getBuiltinModels } from "../node_modules/@earendil-works/pi-ai/dist/providers/all.js";
let count = 0; const gemini = [];
for (const p of getBuiltinProviders()) {
  for (const m of getBuiltinModels(p)) {
    count++;
    if (/gemini|gemma|argon/i.test(m.id)) gemini.push({ provider: p.id, id: m.id, api: m.api, reasoning: m.reasoning, thinking: m.thinkingLevelMap, compat: m.compat && Object.keys(m.compat), baseUrl: m.baseUrl });
  }
}
console.log("total models:", count);
console.log(JSON.stringify(gemini, null, 1).slice(0, 8000));
