import { getBuiltinProviders, getBuiltinModels } from "../node_modules/@earendil-works/pi-ai/dist/providers/all.js";
for (const p of getBuiltinProviders()) {
  if (p.id === "opencode" || p.id === "opencode-go") {
    console.log("PROVIDER", p.id, "baseUrl:", p.baseUrl);
    const models = getBuiltinModels(p);
    console.log("models:", models.length, "sample baseUrl:", JSON.stringify(models.slice(0, 3).map(m => ({ id: m.id, baseUrl: m.baseUrl, api: m.api }))));
  }
}
