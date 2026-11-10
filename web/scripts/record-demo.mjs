#!/usr/bin/env node
// Records the demo fixtures served by the dashboard when no backend is connected
// (static deployments such as Vercel).
//
// A fixture is NOT hand-written: it is the event log of a real run, captured from a
// running OmniForge backend, plus the final workspace. The dashboard replays it through
// the same reducer as a live run.
//
// Usage (backend running, ideally with the Docker sandbox and some simulated latency):
//
//   OMNIFORGE_SIM_DELAY_MS=450 omniforge serve --bind 127.0.0.1:8080
//   node scripts/record-demo.mjs [--api http://127.0.0.1:8080] [--out public/demo]

import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const option = (name, fallback) => {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : fallback;
};
const API = option("api", "http://127.0.0.1:8080").replace(/\/+$/, "");
const OUT = path.resolve(here, "..", option("out", "public/demo"));
const API_KEY = process.env.OMNIFORGE_API_KEY ?? "";

const presets = JSON.parse(await readFile(path.join(here, "../src/lib/presets.json"), "utf8"));
const preset = (id) => presets.find((p) => p.id === id).input;

/** What gets recorded. `decisions` answers gates by stage, in order; default: approve. */
const SCENARIOS = [
  {
    id: "demo-jwt-hitl",
    label: "Micro-tâche sécurisée — supervisée (HITL)",
    description:
      "Un humain valide chaque étape clé : plan (une révision demandée), architecture, plan de test, merge request. On y voit le cycle RED → GREEN → REFACTOR, la boucle d'auto-correction et un correctif de sécurité.",
    request: { input: preset("micro"), supervision: { default_mode: "hitl" } },
    decisions: [
      {
        stage: "plan",
        decision: "request_changes",
        feedback: "Ajoute un critère d'acceptation explicite sur l'expiration du jeton.",
      },
    ],
  },
  {
    id: "demo-prd-autonome",
    label: "Cahier des charges — autonome, merge validée par un humain",
    description:
      "Un PRD décomposé en 7 tâches réparties sur 2 sprints, exécutées sans intervention. La matrice de supervision garde un humain sur la seule merge request finale.",
    request: {
      input: preset("prd"),
      filename: "cahier-des-charges.md",
      supervision: { default_mode: "autonomous", stage_overrides: { merge: "hitl" } },
    },
    decisions: [],
  },
  {
    id: "demo-divergence-hotl",
    label: "Divergence TDD — supervision par exception (HOTL)",
    description:
      "Le développeur n'arrive pas à faire passer les tests. Après 2 itérations, l'orchestrateur lève une exception et rend la main au superviseur, qui arrête le run : rien n'est livré.",
    request: {
      input: "Ajouter un utilitaire de formatage de dates [sim:diverge]",
      supervision: { default_mode: "hotl", max_tdd_iterations: 2 },
    },
    decisions: [
      {
        stage: "implementation",
        decision: "reject",
        feedback: "Le comportement attendu est à préciser avant de poursuivre.",
      },
    ],
  },
];

const headers = { "content-type": "application/json", ...(API_KEY ? { authorization: `Bearer ${API_KEY}` } : {}) };

async function call(method, route, body) {
  const response = await fetch(`${API}${route}`, { method, headers, body: body ? JSON.stringify(body) : undefined });
  const text = await response.text();
  if (!response.ok) throw new Error(`${method} ${route} → HTTP ${response.status}: ${text.slice(0, 200)}`);
  return text ? JSON.parse(text) : null;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const TERMINAL = ["completed", "failed", "cancelled", "interrupted"];

async function record(scenario) {
  const run = await call("POST", "/api/runs", scenario.request);
  const realId = run.id;
  console.log(`  ${scenario.id}: run ${realId} started`);
  const pending = [...scenario.decisions];

  await new Promise((resolve, reject) => {
    const token = API_KEY ? `&token=${encodeURIComponent(API_KEY)}` : "";
    const socket = new WebSocket(`${API.replace(/^http/, "ws")}/api/runs/${realId}/stream?since=0${token}`);
    const deadline = setTimeout(() => reject(new Error(`${scenario.id}: timed out`)), 10 * 60_000);
    socket.onerror = () => reject(new Error(`${scenario.id}: WebSocket error`));
    socket.onmessage = async (message) => {
      const event = JSON.parse(message.data);
      if (event.type === "gate.requested") {
        // Leave the gate open for a moment, as a human would.
        await sleep(1500);
        const index = pending.findIndex((d) => d.stage === event.payload.stage);
        const planned = index >= 0 ? pending.splice(index, 1)[0] : { decision: "approve", feedback: null };
        console.log(`    gate ${event.payload.stage} → ${planned.decision}`);
        await call("POST", `/api/runs/${realId}/gates/${event.payload.gate_id}/resolve`, {
          decision: planned.decision,
          feedback: planned.feedback ?? null,
          resolver: "superviseur",
        });
      }
      if (event.type === "run.status_changed" && TERMINAL.includes(event.payload.status)) {
        clearTimeout(deadline);
        socket.close();
        resolve();
      }
    };
  });
  if (pending.length > 0) throw new Error(`${scenario.id}: planned decisions never used: ${JSON.stringify(pending)}`);

  const [state, events, tree, list] = await Promise.all([
    call("GET", `/api/runs/${realId}`),
    call("GET", `/api/runs/${realId}/events`),
    call("GET", `/api/runs/${realId}/workspace/tree`),
    call("GET", "/api/runs"),
  ]);
  const files = [];
  for (const entry of tree.files) {
    const file = await call("GET", `/api/runs/${realId}/workspace/file?path=${encodeURIComponent(entry.path)}`);
    files.push({ path: entry.path, size: entry.size, content: file.content });
  }
  // Runs that deliver nothing (cancelled, failed) have no merge request diff.
  const diff = await call("GET", `/api/runs/${realId}/workspace/diff`).then((r) => r.diff, () => null);
  const summary = list.runs.find((r) => r.id === realId);

  // Give the recording a stable identity: run ids appear in events, branch names, paths.
  const rename = (value) => JSON.parse(JSON.stringify(value).replaceAll(realId, scenario.id));
  const fixture = rename({ id: scenario.id, state, events: events.events, files, diff });
  console.log(`  ${scenario.id}: ${state.status}, ${fixture.events.length} events, ${files.length} files`);
  return {
    fixture,
    entry: {
      id: scenario.id,
      label: scenario.label,
      description: scenario.description,
      summary: rename(summary),
      event_count: fixture.events.length,
      gate_count: fixture.events.filter((e) => e.type === "gate.requested").length,
    },
  };
}

/** A recording must not leak anything about the machine it was made on. */
function assertPortable(name, value) {
  const text = JSON.stringify(value);
  for (const pattern of [/\/home\/[a-z]/i, /\/Users\//, /\/tmp\/[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+\/runs\//, /\/root\//, /[A-Z]:\\\\/]) {
    const match = text.match(pattern);
    if (match) {
      const at = text.indexOf(match[0]);
      throw new Error(`${name} contains a host path (${text.slice(Math.max(0, at - 60), at + 80)})`);
    }
  }
}

const meta = await call("GET", "/api/meta");
console.log(`Recording from ${API} (llm: ${meta.llm_backend}, sandbox: ${meta.sandbox})`);
if (meta.llm_backend !== "simulated") {
  throw new Error("Demo fixtures are recorded on the simulated backend, so that they can be regenerated identically.");
}
await mkdir(OUT, { recursive: true });

const previews = {};
for (const p of presets) {
  previews[p.id] = { input: p.input, response: await call("POST", "/api/plan", { input: p.input }) };
}

const runs = [];
for (const scenario of SCENARIOS) {
  const { fixture, entry } = await record(scenario);
  assertPortable(scenario.id, fixture);
  await writeFile(path.join(OUT, `${scenario.id}.json`), JSON.stringify(fixture));
  runs.push(entry);
}

const index = { version: 1, recorded_at: new Date().toISOString(), meta, runs, previews };
assertPortable("index", index);
await writeFile(path.join(OUT, "index.json"), JSON.stringify(index));
console.log(`Wrote ${runs.length} recordings and index.json to ${OUT}`);
