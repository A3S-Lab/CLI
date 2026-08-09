document.querySelector('#script-status').textContent = 'Script ready';

let hostPort = null;
const proposeContext = document.querySelector('#propose-context');

function receiveHostInit(event) {
  const message = event.data;
  const port = event.ports[0];
  if (
    event.source !== window.parent ||
    !message ||
    message.protocol !== 'a3s.activity.v2' ||
    message.type !== 'host.init' ||
    !message.payload ||
    typeof message.payload.key !== 'string' ||
    !Number.isSafeInteger(message.payload.generation) ||
    typeof message.payload.revision !== 'string' ||
    !port
  ) {
    return;
  }

  window.removeEventListener('message', receiveHostInit);
  hostPort = port;
  hostPort.start();
  document.querySelector('#broker-status').textContent = 'Broker ready';
  proposeContext.disabled = false;
  hostPort.postMessage({ protocol: 'a3s.activity.v2', type: 'activity.ready' });
}

window.addEventListener('message', receiveHostInit);

proposeContext.addEventListener('click', () => {
  hostPort?.postMessage({
    protocol: 'a3s.activity.v2',
    type: 'context.propose',
    payload: {
      title: 'Review sandbox context',
      summary: 'The sandbox sent this bounded proposal through its dedicated capability port.',
      prompt: 'Confirm that the reviewed sandbox context can be used.',
      fields: [{ label: 'Transport', value: 'MessagePort' }],
      usePackageSkill: false,
    },
  });
});

try {
  localStorage.setItem('a3s-activity-probe', 'denied');
  document.querySelector('#storage-status').textContent = 'Storage accessible';
} catch (_error) {
  document.querySelector('#storage-status').textContent = 'Opaque storage blocked';
}

fetch('/api/v1/plugins/activities')
  .then(() => {
    document.querySelector('#network-status').textContent = 'Network allowed';
  })
  .catch(() => {
    document.querySelector('#network-status').textContent = 'Network blocked';
  });
