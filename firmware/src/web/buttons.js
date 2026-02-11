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

  const readout = document.getElementById("readout");
  const zone_temps = [
    document.getElementById("zone1Temp"),
    document.getElementById("zone2Temp"),
    document.getElementById("zone3Temp"),
  ];
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
          for (let i = 0; i < 3; i++) {
            zone_temps[i].innerHTML = `${data.zone_temps[i]} °C`
          }
        })
        .catch(error => {
          console.error('Fetch error:', error);
        });
    }, 250);
});
