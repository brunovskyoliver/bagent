'use strict';
/**
 * bagent WhatsApp Web bridge
 *
 * Reads from env:
 *   BAGENT_WA_TOKEN    — bearer token (required); all requests must present it
 *   BAGENT_WA_SESSION  — session directory (default ~/Library/Application Support/bagent/whatsapp/session)
 *
 * Binds to 127.0.0.1:0, prints "PORT=<n>" as first stdout line.
 * Never binds 0.0.0.0. Never logs raw message bodies.
 *
 * Security contract (enforced here, not just policy):
 *   - POST /send: exactly one text message per call, no bulk, no media
 *   - Auth header required on every route
 *   - No raw message body in any console.log/error call
 */

const http = require('http');
const os = require('os');
const path = require('path');
const packageInfo = require('./package.json');

// ── Configuration ──────────────────────────────────────────────────────────────

const BEARER_TOKEN = process.env.BAGENT_WA_TOKEN;
if (!BEARER_TOKEN) {
    console.error('[bagent-wa-bridge] BAGENT_WA_TOKEN env is required');
    process.exit(1);
}

const DEFAULT_SESSION_DIR = path.join(
    os.homedir(),
    'Library', 'Application Support', 'bagent', 'whatsapp', 'session'
);
const SESSION_DIR = process.env.BAGENT_WA_SESSION || DEFAULT_SESSION_DIR;

// ── In-memory state ────────────────────────────────────────────────────────────

/** @type {'stopped'|'starting'|'qr'|'authenticated'|'authenticated_waiting_for_ready'|'ready'|'disconnected'|'error'} */
let bridgeStatus = 'starting';
let bridgeError = null;
/** @type {string|null} */
let latestQr = null;
let latestQrUpdatedAt = null;
/** @type {{id:string,name:string|null,pushname:string|null,number:string|null}|null} */
let meInfo = null;
let authenticatedAt = null;
let readyAt = null;
let lastLoading = null;
let lastState = null;
let whatsappWebVersion = null;
let whatsappWebJsVersion = null;
let readyWatchTimer = null;
let pageDiagnosticsAttached = false;
const startedAt = Date.now();

const DIAGNOSTIC_EVENT_LIMIT = 80;
/** @type {Array<Record<string, unknown>>} */
const diagnosticEvents = [];

/** Rolling buffer of recent incoming messages (no raw bodies exposed in logs). */
const RECENT_MSG_LIMIT = 100;
/** @type {Array<{id:string,chatId:string,from:string,to:string|null,body:string,timestamp:number,fromMe:boolean,hasMedia:boolean}>} */
const recentMessages = [];

function sanitizedError(error) {
    const detail = error && error.message ? error.message : String(error || 'unknown');
    return detail.replace(/(Bearer\s+)[A-Za-z0-9._~+/=-]+/gi, '$1[redacted]');
}

function recordEvent(type, fields = {}) {
    const event = {
        at: new Date().toISOString(),
        type,
        ...fields,
    };
    diagnosticEvents.push(event);
    if (diagnosticEvents.length > DIAGNOSTIC_EVENT_LIMIT) {
        diagnosticEvents.shift();
    }
}

function setBridgeStatus(status) {
    bridgeStatus = status;
}

function clearReadyWatch() {
    if (readyWatchTimer) {
        clearTimeout(readyWatchTimer);
        readyWatchTimer = null;
    }
}

function startReadyWatch() {
    clearReadyWatch();
    readyWatchTimer = setTimeout(() => {
        if (bridgeStatus !== 'ready' && bridgeStatus !== 'error' && bridgeStatus !== 'disconnected') {
            setBridgeStatus('authenticated_waiting_for_ready');
            bridgeError = 'Authenticated, but WhatsApp Web has not emitted ready yet';
            recordEvent('ready_timeout', {
                status: bridgeStatus,
                loading_percent: lastLoading ? lastLoading.percent : null,
                loading_message: lastLoading ? lastLoading.message : null,
                state: lastState,
            });
            console.error('[bagent-wa-bridge] authenticated but still waiting for ready');
        }
    }, 90000);
}

function loadWhatsappWebJsVersion() {
    if (whatsappWebJsVersion) return whatsappWebJsVersion;
    try {
        whatsappWebJsVersion = require('whatsapp-web.js/package.json').version || null;
    } catch {
        whatsappWebJsVersion = null;
    }
    return whatsappWebJsVersion;
}

function publicDiagnostics() {
    return {
        status: bridgeStatus,
        error: bridgeError || null,
        latest_qr_updated_at: latestQrUpdatedAt,
        authenticated_at: authenticatedAt,
        ready_at: readyAt,
        last_loading: lastLoading,
        last_state: lastState,
        uptime_ms: Date.now() - startedAt,
        versions: {
            node: process.version,
            bridge: packageInfo.version,
            whatsapp_web_js: loadWhatsappWebJsVersion(),
            whatsapp_web: whatsappWebVersion,
        },
        events: diagnosticEvents.slice(-30),
    };
}

function pushRecentMessage(msg) {
    recentMessages.push(msg);
    if (recentMessages.length > RECENT_MSG_LIMIT) {
        recentMessages.shift();
    }
}

// ── WhatsApp client ────────────────────────────────────────────────────────────

let waClient = null;

const PUPPETEER_ARGS = [
    '--no-sandbox',
    '--disable-setuid-sandbox',
    '--disable-dev-shm-usage',
    '--disable-accelerated-2d-canvas',
    '--no-first-run',
    '--disable-extensions',
    '--disable-gpu',
];

function makeClient(Client, LocalAuth) {
    return new Client({
        authStrategy: new LocalAuth({ dataPath: SESSION_DIR }),
        // webVersionCache type 'none' avoids loading a potentially stale cached
        // HTML that triggers the "Execution context was destroyed" navigation bug.
        webVersionCache: { type: 'none' },
        puppeteer: { headless: true, args: PUPPETEER_ARGS },
    });
}

function attachPageDiagnostics(client) {
    if (pageDiagnosticsAttached || !client || !client.pupPage) return false;
    pageDiagnosticsAttached = true;
    client.pupPage.on('pageerror', (error) => {
        const detail = sanitizedError(error);
        recordEvent('page_error', { detail });
        bridgeError = detail;
        console.error('[bagent-wa-bridge] page_error:', detail);
    });
    client.pupPage.on('requestfailed', (request) => {
        const failure = request.failure();
        const detail = failure && failure.errorText ? failure.errorText : 'request failed';
        const url = request.url();
        recordEvent('request_failed', {
            detail,
            url_host: (() => {
                try { return new URL(url).host; } catch { return null; }
            })(),
        });
    });
    recordEvent('page_diagnostics_attached');
    return true;
}

function attachEvents(client) {
    client.on('qr', (qr) => {
        setBridgeStatus('qr');
        latestQr = qr;
        latestQrUpdatedAt = new Date().toISOString();
        bridgeError = null;
        authenticatedAt = null;
        readyAt = null;
        clearReadyWatch();
        recordEvent('qr', { updated_at: latestQrUpdatedAt });
        // Do NOT log the QR string — it is sensitive auth material.
        console.error('[bagent-wa-bridge] QR generated, waiting for scan');
    });

    client.on('authenticated', () => {
        setBridgeStatus('authenticated');
        latestQr = null;
        bridgeError = null;
        authenticatedAt = new Date().toISOString();
        recordEvent('authenticated');
        startReadyWatch();
        console.error('[bagent-wa-bridge] authenticated');
    });

    client.on('ready', async () => {
        setBridgeStatus('ready');
        bridgeError = null;
        readyAt = new Date().toISOString();
        clearReadyWatch();
        try {
            const info = client.info;
            meInfo = {
                id: info.wid._serialized,
                name: info.pushname || null,
                push_name: info.pushname || null,
                number: info.wid.user || null,
            };
        } catch {
            meInfo = null;
        }
        try {
            whatsappWebVersion = await client.getWWebVersion();
        } catch {
            whatsappWebVersion = null;
        }
        recordEvent('ready', {
            me_present: Boolean(meInfo),
            whatsapp_web: whatsappWebVersion,
        });
        console.error('[bagent-wa-bridge] ready');
    });

    client.on('disconnected', (reason) => {
        setBridgeStatus('disconnected');
        bridgeError = reason || 'disconnected';
        meInfo = null;
        clearReadyWatch();
        recordEvent('disconnected', { reason: reason || null });
        console.error('[bagent-wa-bridge] disconnected:', reason);
    });

    client.on('auth_failure', (msg) => {
        setBridgeStatus('error');
        bridgeError = 'auth_failure: ' + msg;
        clearReadyWatch();
        recordEvent('auth_failure', { detail: 'auth_failure' });
        console.error('[bagent-wa-bridge] auth_failure');
    });

    client.on('loading_screen', (percent, message) => {
        lastLoading = {
            percent: Number(percent),
            message: String(message || ''),
            at: new Date().toISOString(),
        };
        recordEvent('loading_screen', {
            percent: lastLoading.percent,
            message: lastLoading.message,
        });
    });

    client.on('change_state', (state) => {
        lastState = state || null;
        recordEvent('change_state', { state: lastState });
    });

    client.on('message', (msg) => {
        // Store in memory buffer — do NOT log body
        pushRecentMessage({
            id: msg.id._serialized,
            chatId: msg.from,
            from: msg.from,
            to: msg.to || null,
            body: msg.body,             // stored in memory only, never logged
            timestamp: msg.timestamp,
            fromMe: msg.fromMe,
            hasMedia: msg.hasMedia,
        });
    });
}

async function initClientWithRetry(Client, LocalAuth, attempt) {
    const MAX_ATTEMPTS = 3;
    waClient = makeClient(Client, LocalAuth);
    pageDiagnosticsAttached = false;
    attachEvents(waClient);
    const pageTimer = setInterval(() => {
        if (!waClient || attachPageDiagnostics(waClient)) {
            clearInterval(pageTimer);
        }
    }, 500);
    try {
        await waClient.initialize();
        clearInterval(pageTimer);
        attachPageDiagnostics(waClient);
        recordEvent('initialize_resolved');
    } catch (e) {
        clearInterval(pageTimer);
        const detail = e && e.message ? e.message : String(e);
        const isContextError = detail.includes('Execution context was destroyed') ||
                               detail.includes('Session closed') ||
                               detail.includes('Target closed');
        if (isContextError && attempt < MAX_ATTEMPTS) {
            console.error(`[bagent-wa-bridge] initialize failed (attempt ${attempt}/${MAX_ATTEMPTS}): ${detail}`);
            console.error('[bagent-wa-bridge] retrying in 3 s...');
            try { await waClient.destroy(); } catch {}
            waClient = null;
            await new Promise(r => setTimeout(r, 3000));
            return initClientWithRetry(Client, LocalAuth, attempt + 1);
        }
        setBridgeStatus('error');
        bridgeError = 'initialize failed: ' + detail;
        clearReadyWatch();
        recordEvent('initialize_error', { detail: sanitizedError(e) });
        console.error('[bagent-wa-bridge] initialize error:', detail);
        if (e && e.stack) console.error(e.stack);
    }
}

function initClient() {
    let Client, LocalAuth;
    try {
        ({ Client, LocalAuth } = require('whatsapp-web.js'));
        loadWhatsappWebJsVersion();
    } catch (e) {
        setBridgeStatus('error');
        bridgeError = 'whatsapp-web.js not installed — run: npm install in bridge dir';
        recordEvent('dependency_error', { detail: bridgeError });
        console.error('[bagent-wa-bridge] ' + bridgeError);
        return;
    }

    initClientWithRetry(Client, LocalAuth, 1).catch((e) => {
        const detail = e && e.message ? e.message : String(e);
        setBridgeStatus('error');
        bridgeError = 'initialize failed: ' + detail;
        clearReadyWatch();
        recordEvent('fatal_initialize_error', { detail: sanitizedError(e) });
        console.error('[bagent-wa-bridge] fatal initialize error:', detail);
    });
}

// ── HTTP helpers ───────────────────────────────────────────────────────────────

function sendJson(res, status, body) {
    const data = JSON.stringify(body, null, 0);
    res.writeHead(status, {
        'Content-Type': 'application/json; charset=utf-8',
        'Content-Length': Buffer.byteLength(data, 'utf8'),
    });
    res.end(data);
}

function checkAuth(req, res) {
    const auth = req.headers['authorization'] || '';
    if (auth !== 'Bearer ' + BEARER_TOKEN) {
        sendJson(res, 401, { error: 'unauthorized' });
        return false;
    }
    return true;
}

function readBody(req) {
    return new Promise((resolve, reject) => {
        let data = '';
        req.on('data', (chunk) => { data += chunk; });
        req.on('end', () => {
            try { resolve(JSON.parse(data || '{}')); }
            catch (e) { reject(new Error('invalid JSON')); }
        });
        req.on('error', reject);
    });
}

function parseLimit(query, def) {
    const m = (query || '').match(/[?&]limit=(\d+)/);
    return m ? Math.min(parseInt(m[1], 10), 500) : def;
}

function parseBefore(query) {
    const m = (query || '').match(/[?&]before=(\d+)/);
    return m ? parseInt(m[1], 10) : null;
}

// ── Route handlers ────────────────────────────────────────────────────────────

async function handleHealth(req, res) {
    if (!checkAuth(req, res)) return;
    sendJson(res, 200, {
        status: bridgeStatus,
        me: meInfo || undefined,
        error: bridgeError || undefined,
        diagnostics: publicDiagnostics(),
    });
}

async function handleQr(req, res) {
    if (!checkAuth(req, res)) return;
    sendJson(res, 200, {
        qr: latestQr,
        updated_at: latestQrUpdatedAt,
    });
}

async function handleDebug(req, res) {
    if (!checkAuth(req, res)) return;
    sendJson(res, 200, publicDiagnostics());
}

async function handleContacts(req, res, query) {
    if (!checkAuth(req, res)) return;
    if (bridgeStatus !== 'ready' || !waClient) {
        sendJson(res, 503, { error: 'not_ready', status: bridgeStatus });
        return;
    }
    const limit = parseLimit(query, 100);
    try {
        const contacts = await waClient.getContacts();
        const result = contacts.slice(0, limit).map((c) => ({
            id: c.id._serialized,
            name: c.name || null,
            push_name: c.pushname || null,
            phone: c.number || null,
            is_business: c.isBusiness || false,
        }));
        sendJson(res, 200, result);
    } catch (e) {
        sendJson(res, 500, { error: 'contacts_error' });
    }
}

async function handleChats(req, res, query) {
    if (!checkAuth(req, res)) return;
    if (bridgeStatus !== 'ready' || !waClient) {
        sendJson(res, 503, { error: 'not_ready', status: bridgeStatus });
        return;
    }
    const limit = parseLimit(query, 30);
    try {
        const chats = await waClient.getChats();
        const result = chats.slice(0, limit).map((c) => ({
            id: c.id._serialized,
            name: c.name || null,
            is_group: c.isGroup || false,
            unread_count: c.unreadCount || 0,
            timestamp: c.timestamp || null,
            last_message_preview: c.lastMessage
                ? (c.lastMessage.body || '').substring(0, 80)
                : null,
        }));
        sendJson(res, 200, result);
    } catch (e) {
        sendJson(res, 500, { error: 'chats_error' });
    }
}

async function handleChatMessages(req, res, chatId, query) {
    if (!checkAuth(req, res)) return;
    if (bridgeStatus !== 'ready' || !waClient) {
        sendJson(res, 503, { error: 'not_ready', status: bridgeStatus });
        return;
    }
    const limit = parseLimit(query, 20);
    const before = parseBefore(query);
    try {
        const chat = await waClient.getChatById(chatId);
        const msgs = await chat.fetchMessages({ limit });
        const filtered = before
            ? msgs.filter((m) => m.timestamp < before)
            : msgs;
        const result = filtered.map((m) => ({
            id: m.id._serialized,
            chat_id: chatId,
            from: m.from,
            to: m.to || null,
            body: m.body,
            timestamp: m.timestamp,
            from_me: m.fromMe,
            has_media: m.hasMedia,
        }));
        sendJson(res, 200, result);
    } catch (e) {
        sendJson(res, 500, { error: 'messages_error', detail: e && e.message ? e.message : 'unknown' });
    }
}

async function handleSend(req, res) {
    if (!checkAuth(req, res)) return;
    if (bridgeStatus !== 'ready' || !waClient) {
        sendJson(res, 503, { error: 'not_ready', status: bridgeStatus });
        return;
    }
    let body;
    try {
        body = await readBody(req);
    } catch {
        sendJson(res, 400, { error: 'invalid_json' });
        return;
    }

    const text = body.text;
    const chatId = body.chat_id;
    const phone = body.phone;

    if (!text || typeof text !== 'string' || text.trim() === '') {
        sendJson(res, 400, { error: 'text_required' });
        return;
    }
    if (!chatId && !phone) {
        sendJson(res, 400, { error: 'chat_id_or_phone_required' });
        return;
    }
    // No bulk: text must be a single message (no array)
    if (Array.isArray(body.text)) {
        sendJson(res, 400, { error: 'bulk_send_forbidden' });
        return;
    }

    try {
        let targetId = chatId;
        if (!targetId && phone) {
            // Normalise phone: strip non-digits, ensure no leading +
            const digits = phone.replace(/\D/g, '');
            targetId = digits + '@c.us';
        }
        const msg = await waClient.sendMessage(targetId, text.trim());
        // Do NOT log text content
        console.error('[bagent-wa-bridge] sent message id:', msg.id._serialized);
        sendJson(res, 200, {
            ok: true,
            message_id: msg.id._serialized,
        });
    } catch (e) {
        sendJson(res, 500, { error: 'send_error', detail: e && e.message ? e.message : 'unknown' });
    }
}

async function handleLogout(req, res) {
    if (!checkAuth(req, res)) return;
    try {
        if (waClient) {
            await waClient.logout();
        }
        bridgeStatus = 'stopped';
        meInfo = null;
        latestQr = null;
        latestQrUpdatedAt = null;
        authenticatedAt = null;
        readyAt = null;
        clearReadyWatch();
        recordEvent('logout');
        sendJson(res, 200, { ok: true });
        console.error('[bagent-wa-bridge] logged out');
    } catch (e) {
        sendJson(res, 500, { error: 'logout_error' });
    }
}

// ── HTTP server ────────────────────────────────────────────────────────────────

const server = http.createServer(async (req, res) => {
    const url = req.url || '/';
    const [pathPart, queryPart] = url.split('?');
    const query = queryPart ? '?' + queryPart : '';

    // Route: GET /health
    if (req.method === 'GET' && pathPart === '/health') {
        return handleHealth(req, res);
    }
    // Route: GET /qr
    if (req.method === 'GET' && pathPart === '/qr') {
        return handleQr(req, res);
    }
    // Route: GET /debug
    if (req.method === 'GET' && pathPart === '/debug') {
        return handleDebug(req, res);
    }
    // Route: GET /contacts
    if (req.method === 'GET' && pathPart === '/contacts') {
        return handleContacts(req, res, query);
    }
    // Route: GET /chats
    if (req.method === 'GET' && pathPart === '/chats') {
        return handleChats(req, res, query);
    }
    // Route: GET /chats/:id/messages
    const msgMatch = pathPart.match(/^\/chats\/(.+)\/messages$/);
    if (req.method === 'GET' && msgMatch) {
        return handleChatMessages(req, res, decodeURIComponent(msgMatch[1]), query);
    }
    // Route: POST /send
    if (req.method === 'POST' && pathPart === '/send') {
        return handleSend(req, res);
    }
    // Route: POST /logout
    if (req.method === 'POST' && pathPart === '/logout') {
        return handleLogout(req, res);
    }

    sendJson(res, 404, { error: 'not_found' });
});

// Bind to loopback only on a random port; print PORT= as first line.
server.listen(0, '127.0.0.1', () => {
    const port = server.address().port;
    // This MUST be the first stdout line — the Rust connector reads it.
    // After writing it, suppress stdout EPIPE: the daemon closes the read-end
    // of the pipe after reading this line, so any further stdout writes would
    // crash with EPIPE.  All subsequent logging goes to stderr instead.
    process.stdout.write('PORT=' + port + '\n');
    process.stdout.on('error', () => {});  // suppress EPIPE

    console.error('[bagent-wa-bridge] listening on 127.0.0.1:' + port);
    initClient();
});

server.on('error', (e) => {
    console.error('[bagent-wa-bridge] server error:', e.message);
    process.exit(1);
});

// Graceful shutdown
process.on('SIGTERM', async () => {
    try { if (waClient) await waClient.destroy(); } catch {}
    server.close(() => process.exit(0));
});
process.on('SIGINT', async () => {
    try { if (waClient) await waClient.destroy(); } catch {}
    server.close(() => process.exit(0));
});
