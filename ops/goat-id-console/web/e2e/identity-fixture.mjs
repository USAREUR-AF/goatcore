// Local identity-service responses for tests of the console's real templates.
import { createServer } from 'node:http';
const port = Number(process.env.IDENTITY_PORT);
if (!port) throw new Error('IDENTITY_PORT is required');
const identity = {
  id: '00000000-0000-4000-8000-000000000001', schema_id: 'default', state: 'active',
  traits: { email: 'candidate@example.invalid', name: 'Test Candidate' },
  metadata_public: {
    groups: ['GRAFANA-READONLY'],
    org: { short_tag: 'fixture-org' },
    aff: { id: 'fixture-org', kind: 'partner' },
    vendor: { short_tag: 'legacy-fixture-org' },
    unrelated: 'preserved',
  },
  verifiable_addresses: [{ value: 'candidate@example.invalid', verified: true, via: 'email', status: 'completed' }],
};
const initialMetadataPublic = structuredClone(identity.metadata_public);
let revokeCalls = 0;
const heldIdentity = {
  ...identity,
  id: '00000000-0000-4000-8000-000000000002',
  traits: { ...identity.traits, email: 'jane-doe@example.invalid', name: 'Jane Doe' },
  verifiable_addresses: [{ value: 'jane-doe@example.invalid', verified: true, via: 'email', status: 'completed' }],
};
const releaseIdentity = {
  ...identity,
  id: '00000000-0000-4000-8000-000000000003',
  traits: { ...identity.traits, email: 'release-target@example.invalid', name: 'Release Target' },
  verifiable_addresses: [{ value: 'release-target@example.invalid', verified: true, via: 'email', status: 'completed' }],
};
createServer(async (req, res) => {
  const path = new URL(req.url, 'http://localhost').pathname;
  const netbird = path.match(/^\/(zagros|zagros-dev)(\/api\/.*)$/);
  const site = netbird?.[1];
  const apiPath = netbird?.[2];
  res.setHeader('Content-Type', 'application/json');
  if (path === '/healthz') return res.end('{}');
  if (path === '/__fixture/reset' && req.method === 'POST') {
    identity.metadata_public = structuredClone(initialMetadataPublic);
    revokeCalls = 0;
    res.writeHead(204);
    return res.end();
  }
  if (path === '/__fixture/identity' && req.method === 'GET') {
    return res.end(JSON.stringify({ ...identity, revokeCalls }));
  }
  if (apiPath === '/api/peers' && site === 'zagros') return res.end(JSON.stringify([
    {
      id: 'd0f1e2a3b4c5d6e7f8091a2b', name: 'peer-jane-doe-acme-zagros-20260601-laptop',
      ip: '100.127.0.45', connected: true, last_seen: new Date().toISOString(),
      groups: [{ id: 'g-all', name: 'All' }, { id: 'g-probation', name: 'ZAGROS-PROBATION-DEVELOPER' }],
    },
    {
      id: 'b1c2d3e4f5a60718293b4c5d', name: 'peer-release-target-acme-zagros-20260602-laptop',
      ip: '100.127.0.46', connected: true, last_seen: new Date().toISOString(),
      groups: [{ id: 'g-all', name: 'All' }, { id: 'g-probation', name: 'ZAGROS-PROBATION-DEVELOPER' }],
    },
  ]));
  if (apiPath === '/api/peers' && site === 'zagros-dev') return res.end(JSON.stringify([
    {
      id: 'c2d3e4f5a60718293b4c5d6e', name: 'peer-governed-target-acme-zagros-dev-20260603-laptop',
      ip: '100.127.0.47', connected: true, last_seen: new Date().toISOString(),
      groups: [{ id: 'g-all', name: 'All' }, { id: 'g-dev-probation', name: 'ZAGROS-DEV-PROBATION-DEVELOPER' }],
    },
  ]));
  if (apiPath === '/api/groups') return res.end(JSON.stringify([
    { id: 'g-all', name: 'All' },
    { id: 'g-probation', name: 'ZAGROS-PROBATION-DEVELOPER' },
    { id: 'g-quarantined', name: 'ZAGROS-QUARANTINED' },
    { id: 'g-developer', name: 'ZAGROS-DEVELOPER' },
    { id: 'g-p2p', name: 'ZAGROS-P2P' },
    { id: 'g-observer', name: 'ZAGROS-OBSERVER' },
    { id: 'g-dev-probation', name: 'ZAGROS-DEV-PROBATION-DEVELOPER' },
    { id: 'g-dev-developer', name: 'ZAGROS-DEV-DEVELOPER' },
    { id: 'g-dev-p2p', name: 'ZAGROS-DEV-P2P' },
  ]));
  if (apiPath === '/api/setup-keys') return res.end('[]');
  if (path === '/admin/identities' && req.method === 'POST') {
    let text = ''; for await (const chunk of req) text += chunk;
    const body = JSON.parse(text);
    res.writeHead(201);
    return res.end(JSON.stringify({ ...identity, traits: body.traits }));
  }
  if (path === '/admin/identities') return res.end(JSON.stringify([identity]));
  if (path === `/admin/identities/${identity.id}` && req.method === 'PATCH') {
    let text = ''; for await (const chunk of req) text += chunk;
    const ops = JSON.parse(text);
    if (ops.length !== 1 || ops[0].op !== 'add' || ops[0].path !== '/metadata_public/groups' || !Array.isArray(ops[0].value)) {
      res.writeHead(400);
      return res.end(JSON.stringify({ error: { message: 'unsupported patch operation' } }));
    }
    identity.metadata_public ??= {};
    identity.metadata_public.groups = ops[0].value;
    return res.end(JSON.stringify(identity));
  }
  if (path === `/admin/identities/${identity.id}/sessions` && req.method === 'DELETE') {
    revokeCalls++;
    res.writeHead(204);
    return res.end();
  }
  if (path === `/admin/identities/${identity.id}`) return res.end(JSON.stringify(identity));
  if (path === `/admin/identities/${heldIdentity.id}`) return res.end(JSON.stringify(heldIdentity));
  if (path === `/admin/identities/${releaseIdentity.id}`) return res.end(JSON.stringify(releaseIdentity));
  if (path.endsWith('/sessions') || path === '/admin/oauth2/auth/sessions/consent') return res.end('[]');
  if (path === '/admin/recovery/code') return res.end(JSON.stringify({
    recovery_link: 'https://login.example.invalid/recover?flow=fixture',
    recovery_code: '123456', expires_at: '2030-01-01T00:00:00Z',
  }));
  res.writeHead(404); res.end(JSON.stringify({ error: { message: `No test fixture for ${path}` } }));
}).listen(port, '127.0.0.1');
