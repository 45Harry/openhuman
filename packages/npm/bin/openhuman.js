#!/usr/bin/env node
'use strict';

const { spawn, spawnSync, execSync } = require('child_process');
const path = require('path');
const readline = require('readline');
const http = require('http');

const isWin = process.platform === 'win32';
const binName = isWin ? 'openhuman-bin.exe' : 'openhuman-bin';
const binPath = path.join(__dirname, binName);

const CORE_PORT = process.env.OPENHUMAN_CORE_PORT || '7788';
const CORE_HOST = process.env.OPENHUMAN_CORE_HOST || '127.0.0.1';
const RPC_URL = `http://${CORE_HOST}:${CORE_PORT}/rpc`;

function rpcCall(method, params) {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      // Replace every '.' (e.g. nested namespaces) so the method name is built
      // correctly, not just the first separator.
      method: `openhuman.${method.replace(/\./g, '_')}`,
      params: params || {},
    });
    const req = http.request(`${RPC_URL}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
    }, (res) => {
      let data = '';
      res.on('data', (c) => data += c);
      res.on('end', () => {
        try {
          const parsed = JSON.parse(data);
          if (parsed.error) reject(new Error(parsed.error.message || JSON.stringify(parsed.error)));
          else resolve(parsed.result);
        } catch (e) {
          reject(new Error(`parse fail: ${e.message}`));
        }
      });
    });
    req.on('error', reject);
    // Without a timeout, a core that accepts the socket but never responds
    // hangs the chat turn forever. Bound it and surface a clear error so the
    // user can retry.
    req.setTimeout(120000, () => {
      req.destroy(new Error('RPC request timed out after 120s'));
    });
    req.write(body);
    req.end();
  });
}

async function isCoreRunning() {
  return new Promise((resolve) => {
    const req = http.get(`http://${CORE_HOST}:${CORE_PORT}/health`, (res) => {
      resolve(res.statusCode === 200);
    });
    req.on('error', () => resolve(false));
    req.setTimeout(1000, () => { req.destroy(); resolve(false); });
  });
}

async function waitForCore(timeoutMs = 15000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await isCoreRunning()) return true;
    await new Promise(r => setTimeout(r, 300));
  }
  return false;
}

async function runChat(args) {
  // Match the Rust `chat_cli` behavior: print help on -h/--help instead of
  // booting the core and dropping into the REPL.
  if (args.includes('-h') || args.includes('--help')) {
    console.log('');
    console.log(' OpenHuman Chat — interactive coding assistant');
    console.log('');
    console.log(' Usage: openhuman chat [--model <name>]');
    console.log('');
    console.log(' In-session commands:');
    console.log('   /exit, /quit   Quit');
    console.log('   /help          Show commands');
    console.log('');
    return;
  }
  if (!await isCoreRunning()) {
    console.log('[openhuman] Starting core...');
    const core = spawn(binPath, ['run', '--port', CORE_PORT], {
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: true,
    });
    core.stderr.on('data', (d) => {
      const s = d.toString();
      if (process.argv.includes('-v') || process.argv.includes('--verbose')) {
        process.stderr.write(s);
      }
    });
    core.unref();
    if (!await waitForCore()) {
      console.error('Failed to start core. Try: openhuman run');
      process.exit(1);
    }
  }

  const modelIdx = args.indexOf('--model');
  const model = modelIdx !== -1 ? args[modelIdx + 1] : null;

  console.log('');
  console.log(' OpenHuman Chat — interactive coding assistant');
  console.log(' /exit to quit  /help for commands');
  console.log('');

  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    prompt: model ? `oh[${model}]> ` : 'oh> ',
  });

  rl.prompt();

  for await (const line of rl) {
    const trimmed = line.trim();
    if (!trimmed) { rl.prompt(); continue; }

    if (trimmed === '/exit' || trimmed === '/quit') break;
    if (trimmed === '/help') {
      console.log('');
      console.log('  /exit, /quit  Quit');
      console.log('  /help         This help');
      console.log('');
      rl.prompt();
      continue;
    }

    try {
      const result = await rpcCall('agent.chat', {
        message: trimmed,
        model_override: model,
      });
      // Guard the object access: a null/undefined result would otherwise throw
      // "Cannot read properties of null" inside the try and surface as a
      // confusing error rather than an empty response.
      let text;
      if (typeof result === 'string') text = result;
      else if (result && typeof result === 'object') text = result.response || result.result;
      if (text && typeof text === 'object') text = text.response || JSON.stringify(text);
      if (text == null) text = '(no response)';
      console.log('');
      console.log(text);
      console.log('');
    } catch (e) {
      console.error(`Error: ${e.message}`);
    }
    rl.prompt();
  }

  console.log('');
  process.exit(0);
}

async function main() {
  const args = process.argv.slice(2);
  const sub = args[0];

  // Default: `openhuman` with no args starts interactive chat
  if (!sub || sub === 'chat') {
    await runChat(sub ? args.slice(1) : args);
    return;
  }

  // Help flag passes through to the binary (which shows full help)
  if (sub === '--help' || sub === '-h') {
    const result = spawnSync(binPath, args, { stdio: 'inherit', windowsHide: false });
    process.exit(result.status ?? 0);
    return;
  }

  // Pass through to Rust binary
  const result = spawnSync(binPath, args, { stdio: 'inherit', windowsHide: false });

  if (result.error) {
    if (result.error.code === 'ENOENT') {
      process.stderr.write(
        'openhuman binary not found. Try reinstalling: npm install -g openhuman\n'
      );
    } else {
      process.stderr.write(`openhuman: ${result.error.message}\n`);
    }
    process.exit(1);
  }

  process.exit(result.status ?? 0);
}

main().catch((e) => {
  console.error(e.message);
  process.exit(1);
});
