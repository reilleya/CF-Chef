document.addEventListener("DOMContentLoaded", function() {

  const configForm = document.getElementById('configForm');

  configForm.addEventListener('submit', async function(event) {
    event.preventDefault();

    const formData = new FormData(this);

    let enabled_tc_zones = formData.get("use_t1") ? 0x1 : 0x0;
    enabled_tc_zones |= formData.get("use_t2") ? 0x2 : 0x0;
    enabled_tc_zones |= formData.get("use_t3") ? 0x4 : 0x0;

    let enabled_fan_zones = formData.get("use_fan1") ? 0x1 : 0x0;
    enabled_fan_zones |= formData.get("use_fan2") ? 0x2 : 0x0;

    console.log(JSON.stringify({temperature: formData.get("temperature"), time: formData.get("time"), enabled_tc_zones: enabled_tc_zones, enabled_fan_zones: enabled_fan_zones}))

    await fetch("/set_config", {
        method: "POST",
        headers: {
            "Content-Type": "application/json"
        },
        body: JSON.stringify({temperature: Number(formData.get("temperature")), time: Number(formData.get("time")), enabled_tc_zones: enabled_tc_zones, enabled_fan_zones: enabled_fan_zones})
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
          //console.log(data);
          readout.textContent = `${data.current_temp}/${data.setpoint_temp} °C for ${data.run_time_elapsed}/${data.run_time_total} seconds`
          for (let i = 0; i < 3; i++) {
            document.getElementById(`zone${i + 1}Temp`).innerHTML = `${data.temp_zones[i].last_temp} °C`
            document.getElementById(`zone${i + 1}Fault`).innerHTML = `Fault: ${data.temp_zones[i].fault ? 'Yes' : 'No'}`
          }
          for (let i = 0; i < 2; i++) {
            document.getElementById(`fan${i + 1}Speed`).innerHTML = `${data.fans[i].last_speed} RPM`
            document.getElementById(`fan${i + 1}Fault`).innerHTML = `Fault: ${data.fans[i].fault ? 'Yes' : 'No'}`
          }
        })
        .catch(error => {
          console.error('Fetch error:', error);
        });
    }, 250);
});
