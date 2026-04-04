document.addEventListener("DOMContentLoaded", function() {

  const configForm = document.getElementById('configForm');

  configForm.addEventListener('submit', function(event) {
    event.preventDefault();

    const formData = new FormData(this);

    var xhr = new XMLHttpRequest();
    xhr.open("POST", '/set_config', true);

    xhr.setRequestHeader("Content-Type", "application/x-www-form-urlencoded");

    xhr.onreadystatechange = function() {
        if (this.readyState === XMLHttpRequest.DONE && this.status === 200) {
            console.log(this)
        }
    }

    let enabled_tc_zones = formData.get("use_t1") ? 0x1 : 0x0;
    enabled_tc_zones |= formData.get("use_t2") ? 0x2 : 0x0;
    enabled_tc_zones |= formData.get("use_t3") ? 0x4 : 0x0;

    let enabled_fan_zones = formData.get("use_fan1") ? 0x1 : 0x0;
    enabled_fan_zones |= formData.get("use_fan2") ? 0x2 : 0x0;


    let request =
      "temperature=" + formData.get("temperature") + "&" +
      "time=" + formData.get("time") + "&" +
      "enabled_tc_zones=" + enabled_tc_zones + "&" +
      "enabled_fan_zones=" + enabled_fan_zones;

    console.log(request);

    xhr.send(request);
  });

  const readout = document.getElementById("readout");
  const zone_temps = [
    document.getElementById("zone1Temp"),
    document.getElementById("zone2Temp"),
    document.getElementById("zone3Temp"),
  ];
  const fan_speeds = [
    document.getElementById("fan1Speed"),
    document.getElementById("fan2Speed"),
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
          console.log(data);
          readout.textContent = `${data.current_temp}/${data.setpoint_temp} °C for ${data.run_time_elapsed}/${data.run_time_total} seconds`
          for (let i = 0; i < 3; i++) {
            zone_temps[i].innerHTML = `${data.temp_zones[i].last_temp} °C`
          }
          for (let i = 0; i < 2; i++) {
            fan_speeds[i].innerHTML = `${data.fans[i].last_speed} RPM`
          }
        })
        .catch(error => {
          console.error('Fetch error:', error);
        });
    }, 250);
});
