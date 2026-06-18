'use strict';
/**
 * Bridge unit tests — run with: node test.js
 * Does NOT require whatsapp-web.js to be installed.
 * Does NOT connect to WhatsApp.
 */

let passed = 0;
let failed = 0;

function assert(condition, label) {
    if (condition) {
        console.log('  ✓', label);
        passed++;
    } else {
        console.error('  ✗', label);
        failed++;
    }
}

// ── 1. Slovak UTF-8 roundtrip ─────────────────────────────────────────────────

console.log('\n1. Slovak UTF-8 roundtrip');
const sk = 'Dobrý deň, posielam faktúru č. 2026/001. Ďakujem.';
const encoded = Buffer.from(JSON.stringify({ text: sk }), 'utf8');
const decoded = JSON.parse(encoded.toString('utf8'));
assert(decoded.text === sk, 'Slovak text roundtrips through JSON/Buffer unchanged');
assert(decoded.text.includes('Ď'), 'Ď (U+010E) preserved');
assert(decoded.text.includes('č'), 'č (U+010D) preserved');
assert(decoded.text.includes('ú'), 'ú (U+00FA) preserved');
assert(decoded.text.includes('ň'), 'ň (U+0148) preserved');

// ── 2. Auth header check ─────────────────────────────────────────────────────

console.log('\n2. Auth header check');
const TOKEN = 'test-token-abc123';
function checkAuth(authHeader) {
    return (authHeader || '') === 'Bearer ' + TOKEN;
}
assert(!checkAuth(null), 'null auth rejected');
assert(!checkAuth(''), 'empty auth rejected');
assert(!checkAuth('Bearer wrong'), 'wrong token rejected');
assert(!checkAuth('Bearer '), 'empty bearer rejected');
assert(!checkAuth(TOKEN), 'bare token without Bearer rejected');
assert(checkAuth('Bearer ' + TOKEN), 'correct Bearer accepted');

// ── 3. Send payload validation ────────────────────────────────────────────────

console.log('\n3. Send payload validation');
function validateSend(body) {
    if (!body.text || typeof body.text !== 'string' || body.text.trim() === '') {
        return 'text_required';
    }
    if (Array.isArray(body.text)) {
        return 'bulk_send_forbidden';
    }
    if (!body.chat_id && !body.phone) {
        return 'chat_id_or_phone_required';
    }
    return null;
}
assert(validateSend({}) === 'text_required', 'missing text rejected');
assert(validateSend({ text: '' }) === 'text_required', 'empty text rejected');
assert(validateSend({ text: '  ' }) === 'text_required', 'whitespace-only text rejected');
assert(validateSend({ text: sk }) === 'chat_id_or_phone_required', 'missing target rejected');
assert(validateSend({ text: sk, chat_id: '1234@c.us' }) === null, 'valid chat_id accepted');
assert(validateSend({ text: sk, phone: '+421901234567' }) === null, 'valid phone accepted');

// ── 4. Status serialization ────────────────────────────────────────────────────

console.log('\n4. Status serialization');
const validStatuses = ['stopped', 'starting', 'qr', 'authenticated', 'authenticated_waiting_for_ready', 'ready', 'disconnected', 'error'];
for (const s of validStatuses) {
    const json = JSON.stringify({ status: s, me: null, error: undefined });
    const obj = JSON.parse(json);
    assert(obj.status === s, `status "${s}" serializes/deserializes correctly`);
}

// ── 5. Phone normalization ────────────────────────────────────────────────────

console.log('\n5. Phone normalization');
function normalizePhone(phone) {
    const digits = phone.replace(/\D/g, '');
    return digits + '@c.us';
}
assert(normalizePhone('+421901234567') === '421901234567@c.us', 'SK number with + normalized');
assert(normalizePhone('0901 234 567') === '0901234567@c.us', 'spaces stripped');
assert(normalizePhone('421901234567') === '421901234567@c.us', 'no-plus number normalized');

// ── Summary ────────────────────────────────────────────────────────────────────

console.log(`\n${passed + failed} tests: ${passed} passed, ${failed} failed\n`);
if (failed > 0) {
    process.exit(1);
}
