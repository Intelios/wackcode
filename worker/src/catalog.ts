process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_OFFLINE = "1";

// Load only the catalogue. This process never receives a provider key or starts a Pi session.
const { listBuiltinModelSuggestions } = await import("./catalog-models.js");
process.stdout.write(JSON.stringify(listBuiltinModelSuggestions()));
