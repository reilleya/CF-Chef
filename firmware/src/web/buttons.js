let schedule = [];

function renderSchedule() {
  const tbody = document.getElementById('scheduleBody');
  tbody.innerHTML = '';
  schedule.forEach(function(step, i) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td>${i + 1}</td>
      <td><select onchange="schedule[${i}].ramp = this.value === 'ramp'">
        <option value="hold" ${!step.ramp ? 'selected' : ''}>Hold</option>
        <option value="ramp" ${step.ramp ? 'selected' : ''}>Ramp</option>
      </select></td>
      <td><input type="number" min="1" value="${step.duration}"
          onchange="schedule[${i}].duration = Number(this.value)"></td>
      <td><input type="number" value="${step.temperature}"
          onchange="schedule[${i}].temperature = Number(this.value)"></td>
      <td><button type="button" onclick="deleteStep(${i})">✕</button></td>`;
    tbody.appendChild(tr);
  });
  document.getElementById('stepCount').textContent = `${schedule.length} / 32 steps`;
  document.getElementById('addStepBtn').disabled = schedule.length >= 32;
}

function addStep() {
  if (schedule.length >= 32) return;
  schedule.push({duration: 60, temperature: 25, ramp: false});
  renderSchedule();
}

function deleteStep(i) {
  schedule.splice(i, 1);
  renderSchedule();
}

document.addEventListener("DOMContentLoaded", function() {
  document.getElementById('addStepBtn').addEventListener('click', addStep);

  const configForm = document.getElementById('configForm');

  configForm.addEventListener('submit', async function(event) {
    event.preventDefault();
    const formData = new FormData(this);

    let enabled_tc_zones  = formData.get("use_t1")   ? 0x1 : 0;
    enabled_tc_zones     |= formData.get("use_t2")   ? 0x2 : 0;
    enabled_tc_zones     |= formData.get("use_t3")   ? 0x4 : 0;
    let enabled_fan_zones  = formData.get("use_fan1") ? 0x1 : 0;
    enabled_fan_zones    |= formData.get("use_fan2") ? 0x2 : 0;

    const padded = schedule.slice(0, 32);
    while (padded.length < 32) padded.push({duration: 0, temperature: 0, ramp: false});

    await fetch("/set_config", {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({enabled_tc_zones, enabled_fan_zones, schedule: padded})
    });
  });

  let statusUpdateInterval = setInterval(function() {
      fetch('/get_state')
        .then(response => {
          if (!response.ok) {
            throw new Error(`HTTP error! status: ${response.status}`);
          }
          return response.json();
        })
        .then(data => {
          document.getElementById('tempReadout').textContent = `${data.current_temp} / ${data.current_setpoint} °C`;
          document.getElementById('timeReadout').textContent = `${data.run_time_elapsed} / ${data.run_time_total} s`;
          const stateNames = ['Configuration', 'Running', 'Complete', 'Error'];
          let stateText = stateNames[data.run_state] ?? 'Unknown';
          document.getElementById('machineState').textContent = stateText;
          for (let i = 0; i < 3; i++) {
            document.getElementById(`zone${i + 1}Temp`).innerHTML = data.temp_zones[i].fault ? '-' : `${data.temp_zones[i].last_temp} °C`;
            document.getElementById(`zone${i + 1}Fault`).innerHTML = `Fault: ${data.temp_zones[i].fault ? 'Yes' : 'No'}`;
          }
          for (let i = 0; i < 2; i++) {
            document.getElementById(`fan${i + 1}Speed`).innerHTML = data.fans[i].fault ? '-' : `${data.fans[i].last_speed} RPM`;
            document.getElementById(`fan${i + 1}Fault`).innerHTML = `Fault: ${data.fans[i].fault ? 'Yes' : 'No'}`;
          }
        })
        .catch(error => {
          console.error('Fetch error:', error);
        });
    }, 250);
});
