document.querySelector('#script-status').textContent = 'Script ready';

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
