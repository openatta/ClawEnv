// Copy clawenv-cli binary to tauri/binaries/ with target-triple suffix
// Usage: node scripts/copy-cli-sidecar.js [release|debug]

const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const profile = process.argv[2] || "release";
const triple = execSync("rustc -vV").toString().match(/host: (.+)/)[1].trim();
const ext = process.platform === "win32" ? ".exe" : "";

const src = path.join("target", profile, `clawenv-cli${ext}`);
const destDir = path.join("tauri", "binaries");
const dest = path.join(destDir, `clawenv-cli-${triple}${ext}`);

fs.mkdirSync(destDir, { recursive: true });
fs.copyFileSync(src, dest);
console.log(`Copied ${src} -> ${dest}`);
