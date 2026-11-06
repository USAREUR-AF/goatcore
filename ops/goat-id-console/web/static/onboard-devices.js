// #2384: fetch only the selected person's devices, after the page is usable.
// Stage lists and sponsor cards have no device placeholder and make no request.
(function () {
  document.querySelectorAll('[data-devices-url]').forEach(function (el) {
    function load() {
      el.setAttribute('aria-busy', 'true');
      var message = document.createElement('p');
      message.className = 'ob-note dim';
      message.textContent = 'Loading mesh devices…';
      el.replaceChildren(message);

      fetch(el.dataset.devicesUrl, { credentials: 'same-origin', cache: 'no-store' })
        .then(function (response) {
          // A session expiring can redirect to login with a successful HTML
          // response. Keep that page out of the candidate's device panel.
          if (!response.ok || response.redirected ||
              !(response.headers.get('content-type') || '').startsWith('text/html')) {
            throw new Error('Device lookup unavailable');
          }
          return response.text();
        })
        .then(function (html) {
          // Same-origin, admin-gated fragment; all peer fields are escaped by
          // the server's html/template, using the same block as the card.
          el.innerHTML = html;
          el.setAttribute('aria-busy', 'false');
        })
        .catch(function () {
          message.textContent = 'Mesh devices could not be loaded. ';
          var retry = document.createElement('button');
          retry.type = 'button';
          retry.textContent = 'Retry';
          retry.addEventListener('click', load);
          message.appendChild(retry);
          el.replaceChildren(message);
          el.setAttribute('aria-busy', 'false');
        });
    }
    load();
  });
})();
