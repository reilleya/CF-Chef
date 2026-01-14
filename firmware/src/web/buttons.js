document.addEventListener("DOMContentLoaded", function() {

  const configForm = document.getElementById('configForm');

  configForm.addEventListener('submit', function(event) {
    event.preventDefault();

    const formData = new FormData(this);

    var xhr = new XMLHttpRequest();
    xhr.open("POST", '/set_config', true);

    xhr.setRequestHeader("Content-Type", "application/x-www-form-urlencoded");

    xhr.onreadystatechange = function() { // Call a function when the state changes.
        if (this.readyState === XMLHttpRequest.DONE && this.status === 200) {
            console.log(this)
        }
    }
    let request = "";
    for (const p of formData) {
      request += `${p[0]}=${p[1]}&`
    }
    request = request.slice(0, -1);

    xhr.send(request);
  });

  const incrementButton = document.getElementById("increment");
  incrementButton.addEventListener("click", function() {
      fetch('/increment')
        .then(response => {
          if (!response.ok) {
            throw new Error(`HTTP error! status: ${response.status}`);
          }
          return void 0;
        }).catch(error => {
          console.error('Fetch error:', error);
        });
  });

  const readout = document.getElementById("readout");
  let readoutInterval = setInterval(function() {
      fetch('/get_state')
        .then(response => {
          if (!response.ok) {
            throw new Error(`HTTP error! status: ${response.status}`);
          }
          return response.json();
        })
        .then(data => {
          readout.textContent = `${data.current_temp}/${data.setpoint_temp} °C for ${data.run_time_elapsed}/${data.run_time_total} seconds`
        })
        .catch(error => {
          console.error('Fetch error:', error);
        });
    }, 250);
});