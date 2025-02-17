import math
import serial
import sys
import time
import json

port = "hwgrep://0001"
print(f'Using port {port}')

ser = serial.serial_for_url(port, 230400, timeout=2)

ENUMERATE = 'E'
SET_COLORS = 'L'
SET_MAPPING = 'M'

num_panels = 1

class TimeoutError(Exception):
	pass

def send_command(cmd_byte, params = ""):
	cmd_bytes = bytes(f"{cmd_byte}{params}\n", "ascii")
	# print("> ", cmd_bytes)
	ser.write(cmd_bytes)
	response = ser.read_until(b"\n")
	# print("< ", response, "\n")

	if len(response) == 0 :#or response[len(response)-1] != 10:
		raise TimeoutError("Timeout")

	return response

def send_enumerate():
	response_data = send_command(ENUMERATE)
	global num_panels
	response_str = response_data.decode("ascii", errors="replace").strip()
	try:
		response = json.loads(response_str)
	except Exception as e:
		print(f"nope: {e}")
		response = []
	num_panels = len(response)
	return response

def send_set_mapping(ids):
	response = send_command(SET_MAPPING, "".join((f'{id:02x}' for id in ids)))
	return response

# lights = [id0, R0, G0, B0, id1, R1, G1, B1, ...]
#
def send_set_colors(colors):
	response_data = send_command(SET_COLORS, "".join((f'{c:02x}' for c in colors)))
	response_str = response_data.decode("ascii", errors="replace").strip()
	# print(f">>>{response}")
	return response_str

def wave(period, pos):
	return int(math.sin((pos / period) * 2*math.pi) * 127) + 128

panels = []

def do_enumerate():
	global panels
	response = send_enumerate()
	for panel in response:
		print(panel)
	panels = [p["id"] for p in response]
	if len(panels) > 20:
		print("Too many panels")
		panels = panels[:20]
	send_set_mapping(panels)

def test():
	# while True:
	period = 500
	i = 0
	while True:
		# send_enumerate()
		# time.sleep(0.02)
		# continue

		if (i % 100) == 0:
			do_enumerate()
			print("")

		colors = []
		for n, _ in enumerate(panels):
			rgb = [wave(period, i), wave(period/3, i + n*50), wave(period*2, i + n*100)]
			colors.extend(rgb)
		response = send_set_colors(colors)
		ansi_clear_line = "\x1b[2K"
		ansi_cursor_up = "\x1b[A"
		print(f"{ansi_cursor_up}{ansi_clear_line}PIRS: {str(response)}")
		i += 1
		time.sleep(0.02)

def test_forever():
	while True:
		try:
			test();
		except TimeoutError:
			print("TIMEOUT")
			pass

if __name__ == "__main__":
	test_forever()
