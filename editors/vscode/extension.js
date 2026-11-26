// Skol VS Code extension: runs `skol check` on .skl files and shows
// parse errors as squiggles + entries in the Problems panel. No npm
// dependencies — the `vscode` module is supplied by VS Code at runtime.
const vscode = require("vscode");
const cp = require("child_process");
const fs = require("fs");
const path = require("path");

const LINE_RE = /^(.*):(\d+):(\d+): (error|warning): (.*)$/;
let diagnostics;
let output;
const timers = new Map();
let warnedMissing = false;

function activate(context) {
  diagnostics = vscode.languages.createDiagnosticCollection("skol");
  output = vscode.window.createOutputChannel("Skol");
  context.subscriptions.push(diagnostics, output);

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(check),
    vscode.workspace.onDidSaveTextDocument(check),
    vscode.workspace.onDidChangeTextDocument((e) => {
      if (!config().get("checkOnType")) return;
      schedule(e.document);
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => diagnostics.delete(doc.uri)),
    vscode.commands.registerCommand("skol.check", () => {
      const doc = activeSkolDoc();
      if (doc) check(doc, true);
    }),
    vscode.commands.registerCommand("skol.showAst", () => dump("ast")),
    vscode.commands.registerCommand("skol.showTokens", () => dump("tokens")),
    vscode.commands.registerCommand("skol.showIr", () => dump("ir")),
    vscode.commands.registerCommand("skol.showBytecode", () => dump("disasm")),
    vscode.commands.registerCommand("skol.run", runFile)
  );

  vscode.workspace.textDocuments.forEach(check);
}

function config() {
  return vscode.workspace.getConfiguration("skol");
}

function activeSkolDoc() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.document.languageId !== "skol") {
    vscode.window.showWarningMessage("Open a .skl file first.");
    return undefined;
  }
  return ed.document;
}

function findSkol(doc) {
  const configured = config().get("executablePath");
  if (configured) return configured;
  const exe = process.platform === "win32" ? "skol.exe" : "skol";
  const folders = [];
  const wf = vscode.workspace.getWorkspaceFolder(doc.uri);
  if (wf) folders.push(wf.uri.fsPath);
  (vscode.workspace.workspaceFolders || []).forEach((f) => folders.push(f.uri.fsPath));
  // Prefer whichever build (debug or release) was produced most recently.
  for (const root of folders) {
    let best = null;
    let bestTime = -1;
    for (const profile of ["debug", "release"]) {
      const p = path.join(root, "target", profile, exe);
      try {
        const t = fs.statSync(p).mtimeMs;
        if (t > bestTime) { best = p; bestTime = t; }
      } catch (_) { /* not built */ }
    }
    if (best) return best;
  }
  return "skol"; // fall back to PATH
}

function run(doc, cmd, cb) {
  const skol = findSkol(doc);
  const name = doc.uri.scheme === "file" ? doc.uri.fsPath : doc.uri.toString();
  const child = cp.execFile(skol, [cmd, "-", "--name", name], { maxBuffer: 32 * 1024 * 1024 }, (err, stdout, stderr) => {
    if (err && err.code === "ENOENT") {
      if (!warnedMissing) {
        warnedMissing = true;
        vscode.window.showErrorMessage(
          `Skol: couldn't find the skol executable ('${skol}'). Run 'cargo build' in your skol folder, or set skol.executablePath.`
        );
      }
      return;
    }
    cb(stdout, stderr, err);
  });
  child.stdin.end(doc.getText());
}

function schedule(doc) {
  if (doc.languageId !== "skol") return;
  const key = doc.uri.toString();
  clearTimeout(timers.get(key));
  timers.set(key, setTimeout(() => check(doc), 300));
}

function check(doc, announce) {
  if (doc.languageId !== "skol") return;
  run(doc, "check", (stdout, stderr) => {
    const diags = [];
    for (const line of stdout.split(/\r?\n/)) {
      const m = LINE_RE.exec(line);
      if (!m) continue;
      const ln = Math.max(0, parseInt(m[2], 10) - 1);
      const col = Math.max(0, parseInt(m[3], 10) - 1);
      const pos = new vscode.Position(Math.min(ln, doc.lineCount - 1), col);
      const range = doc.getWordRangeAtPosition(pos) || new vscode.Range(pos, pos.translate(0, 1));
      const sev = m[4] === "warning" ? vscode.DiagnosticSeverity.Warning : vscode.DiagnosticSeverity.Error;
      const d = new vscode.Diagnostic(range, m[5], sev);
      d.source = "skol";
      diags.push(d);
    }
    diagnostics.set(doc.uri, diags);
    if (stderr.trim()) output.appendLine(stderr.trim());
    if (announce) {
      vscode.window.setStatusBarMessage(diags.length ? `Skol: ${diags.length} error(s)` : "Skol: no errors", 4000);
    }
  });
}

function dump(cmd) {
  const doc = activeSkolDoc();
  if (!doc) return;
  run(doc, cmd, async (stdout, stderr) => {
    const content = stdout || stderr;
    const view = await vscode.workspace.openTextDocument({ content, language: cmd === "ast" ? "rust" : "plaintext" });
    vscode.window.showTextDocument(view, vscode.ViewColumn.Beside, true);
  });
}

let terminal;

async function runFile() {
  const doc = activeSkolDoc();
  if (!doc) return;
  if (doc.isDirty) await doc.save();
  if (doc.uri.scheme !== "file") {
    vscode.window.showWarningMessage("Save the file to disk before running it.");
    return;
  }
  const skol = findSkol(doc);
  if (!terminal || terminal.exitStatus !== undefined) {
    terminal = vscode.window.createTerminal("Skol");
  }
  terminal.show(true);
  const q = (s) => (s.includes(" ") ? `"${s}"` : s);
  const prefix = process.platform === "win32" && skol.includes(" ") ? "& " : "";
  terminal.sendText(`${prefix}${q(skol)} run ${q(doc.uri.fsPath)}`);
}

function deactivate() {}

module.exports = { activate, deactivate };
