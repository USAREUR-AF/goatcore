"""
Selenium tests for Air Defenders proximity blast multi-kill behavior.

Validates that armed interceptor detonation kills ALL targets within
PROX_RADIUS, not just the trigger target. Uses window._testStep() to
advance the simulation deterministically without waiting for real-time
requestAnimationFrame.

Run:
    pip install selenium pytest
    pytest ops/serve/tests/test_blast.py -v

Requires: Chrome/Chromium + chromedriver (or set SELENIUM_BROWSER=firefox)
"""

import os
import time
import threading
import http.server
import pytest
from pathlib import Path
from selenium import webdriver
from selenium.webdriver.chrome.options import Options as ChromeOptions
from selenium.webdriver.firefox.options import Options as FirefoxOptions

SERVE_DIR = str(Path(__file__).resolve().parent.parent / "airdefenders")
GAME_FILE = "index.html"


class GameServer:
    """Simple HTTP server to serve the game file."""

    def __init__(self):
        self.port = 0
        self.httpd = None
        self.thread = None

    def start(self):
        handler = http.server.SimpleHTTPRequestHandler
        self.httpd = http.server.HTTPServer(("127.0.0.1", 0), handler)
        self.port = self.httpd.server_address[1]
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()

    def stop(self):
        if self.httpd:
            self.httpd.shutdown()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.port}/{GAME_FILE}"


@pytest.fixture(scope="module")
def server():
    srv = GameServer()
    old_cwd = os.getcwd()
    os.chdir(SERVE_DIR)
    srv.start()
    yield srv
    srv.stop()
    os.chdir(old_cwd)


@pytest.fixture(scope="module")
def driver():
    browser = os.environ.get("SELENIUM_BROWSER", "chrome").lower()
    if browser == "firefox":
        opts = FirefoxOptions()
        opts.add_argument("--headless")
        d = webdriver.Firefox(options=opts)
    else:
        opts = ChromeOptions()
        opts.add_argument("--headless=new")
        opts.add_argument("--no-sandbox")
        opts.add_argument("--disable-gpu")
        d = webdriver.Chrome(options=opts)
    yield d
    d.quit()


def setup_combat_state(driver):
    """Put the game into COMBAT state with no spawning.
    Sets _testMode to prevent the rAF game loop from calling updateCombat,
    so only _testStep advances the simulation deterministically."""
    driver.execute_script("""
        window._testMode = true;
        state = 2;  // COMBAT
        wave = 1;
        score = 0;
        lives = 5;
        shaheds = [];
        interceptors = [];
        activeIdx = -1;
        shahedsToSpawn = 0;
        spawnTimer = 0;
        radars = [];
        explosions = [];
        blastWaves = [];
    """)


def add_stationary_shahed(driver, x, y, cruise=False):
    """Inject a stationary shahed at the given position."""
    driver.execute_script(f"""
        shaheds.push({{
            x: {x}, y: {y}, baseX: {x}, speed: 0,
            pattern: 4, amplitude: 0, frequency: 0, phase: 0, wobble: 0,
            cruise: {'true' if cruise else 'false'}, t: 0, alive: true,
            angle: Math.PI, visible: 1, path: [], outcome: null
        }});
    """)


def add_interceptor(driver, x, y, armed=True, auto_pilot=False):
    """Inject an interceptor at the given position heading upward."""
    driver.execute_script(f"""
        interceptors.push({{
            x: {x}, y: {y}, vx: 0, vy: -3.5,
            alive: true, armed: {'true' if armed else 'false'},
            autoPilot: {'true' if auto_pilot else 'false'},
            trail: [], age: 0, path: [], outcome: null
        }});
        activeIdx = interceptors.length - 1;
    """)


def step_frames(driver, n=1):
    """Advance the combat simulation by n frames."""
    driver.execute_script(f"window._testStep({n});")


def get_state(driver):
    """Read back game state as a dict."""
    return driver.execute_script("""
        return {
            score: score,
            lives: lives,
            shaheds: shaheds.map(s => ({
                alive: s.alive, outcome: s.outcome, x: s.x, y: s.y
            })),
            interceptors: interceptors.map(i => ({
                alive: i.alive, outcome: i.outcome, x: i.x, y: i.y, armed: i.armed
            })),
            explosionCount: explosions.length,
            blastWaveCount: blastWaves.length
        };
    """)


class TestProximityBlastMultiKill:
    """Armed interceptor detonation must kill all targets within PROX_RADIUS."""

    def test_two_shaheds_within_blast_radius(self, server, driver):
        """Two shaheds 40px apart should both die from one armed blast."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        # Place two shaheds close together (40px apart horizontally)
        add_stationary_shahed(driver, 400, 200)
        add_stationary_shahed(driver, 440, 200)
        # Armed interceptor between them, close enough to trigger
        # Distance to shahed 0: sqrt(20^2 + 15^2) = 25 < PROX_TRIGGER (36.75)
        add_interceptor(driver, 420, 215, armed=True)

        step_frames(driver, 5)
        st = get_state(driver)

        assert not st["shaheds"][0]["alive"], "First shahed should be dead"
        assert not st["shaheds"][1]["alive"], "Second shahed should be dead (multi-kill)"
        assert st["shaheds"][0]["outcome"] == "intercepted"
        assert st["shaheds"][1]["outcome"] == "intercepted"
        assert not st["interceptors"][0]["alive"], "Interceptor should be consumed"
        assert st["interceptors"][0]["outcome"] == "hit"
        # Base score: 2 * 100 = 200, multi-kill bonus: 2 * 75 = 150 → 350
        assert st["score"] >= 350, f"Expected multi-kill score >= 350, got {st['score']}"
        assert st["blastWaveCount"] >= 1, "Should have spawned a blast wave"

    def test_three_shaheds_multi_kill(self, server, driver):
        """Three shaheds in a cluster should all die from one blast."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        add_stationary_shahed(driver, 380, 200)
        add_stationary_shahed(driver, 400, 190)
        add_stationary_shahed(driver, 420, 200)
        # Interceptor in the center, within trigger range of middle shahed
        add_interceptor(driver, 400, 210, armed=True)

        step_frames(driver, 5)
        st = get_state(driver)

        dead_count = sum(1 for s in st["shaheds"] if not s["alive"])
        assert dead_count == 3, f"Expected 3 kills, got {dead_count}"
        # 3 * 100 base + 3 * 75 multi-kill = 525
        assert st["score"] >= 525, f"Expected score >= 525, got {st['score']}"

    def test_shahed_outside_blast_radius_survives(self, server, driver):
        """A shahed beyond PROX_RADIUS should survive the blast."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        add_stationary_shahed(driver, 400, 200)  # near interceptor
        add_stationary_shahed(driver, 600, 200)  # 200px away > PROX_RADIUS (147)
        add_interceptor(driver, 400, 215, armed=True)

        step_frames(driver, 5)
        st = get_state(driver)

        assert not st["shaheds"][0]["alive"], "Near shahed should be dead"
        assert st["shaheds"][1]["alive"], "Far shahed should survive"
        assert st["shaheds"][1]["outcome"] is None

    def test_unarmed_only_kills_direct_hit(self, server, driver):
        """Unarmed interceptor should only kill by direct contact, not proximity."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        add_stationary_shahed(driver, 400, 200)
        add_stationary_shahed(driver, 420, 200)  # 20px away — within PROX but not direct hit
        # Unarmed interceptor aimed directly at first shahed
        add_interceptor(driver, 400, 215, armed=False)

        step_frames(driver, 10)
        st = get_state(driver)

        assert not st["shaheds"][0]["alive"], "Direct-hit shahed should be dead"
        assert st["shaheds"][1]["alive"], "Nearby shahed should survive unarmed hit"

    def test_cruise_missile_multi_kill_scoring(self, server, driver):
        """Cruise missiles in a multi-kill should score 250 each + multi-kill bonus."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        add_stationary_shahed(driver, 400, 200, cruise=True)
        add_stationary_shahed(driver, 420, 200, cruise=True)
        add_interceptor(driver, 410, 215, armed=True)

        step_frames(driver, 5)
        st = get_state(driver)

        dead_count = sum(1 for s in st["shaheds"] if not s["alive"])
        assert dead_count == 2, f"Expected 2 kills, got {dead_count}"
        # 2 * 250 base + 2 * 75 multi-kill = 650
        assert st["score"] >= 650, f"Expected score >= 650, got {st['score']}"


class TestInterceptorControl:
    """Most recently fired interceptor should be the active (controlled) one."""

    def test_newest_interceptor_is_active(self, server, driver):
        """Firing a second interceptor should make it the active one."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        add_interceptor(driver, 300, 600, armed=False)
        add_interceptor(driver, 500, 600, armed=False)

        idx = driver.execute_script("return activeIdx;")
        assert idx == 1, f"Expected activeIdx=1 (newest), got {idx}"

    def test_switch_picks_newest_alive_on_death(self, server, driver):
        """When active interceptor dies, switchInterceptor should pick newest alive."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        # Three interceptors
        add_interceptor(driver, 200, 500, armed=False)
        add_interceptor(driver, 400, 500, armed=False)
        add_interceptor(driver, 600, 500, armed=False)

        # Kill the newest (index 2)
        driver.execute_script("""
            interceptors[2].alive = false;
            switchInterceptor();
        """)
        idx = driver.execute_script("return activeIdx;")
        assert idx == 1, f"Expected activeIdx=1 (next newest alive), got {idx}"

    def test_non_active_interceptors_continue_trajectory(self, server, driver):
        """Non-active interceptors should continue moving on their trajectory."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        add_interceptor(driver, 300, 600, armed=False)
        initial_y = driver.execute_script("return interceptors[0].y;")
        add_interceptor(driver, 500, 600, armed=False)

        step_frames(driver, 10)

        y_after = driver.execute_script("return interceptors[0].y;")
        assert y_after < initial_y, f"Non-active interceptor should have moved up: was {initial_y}, now {y_after}"

    def test_autopilot_seeks_on_non_active(self, server, driver):
        """Non-active autopilot interceptor should still seek targets."""
        driver.get(server.url)
        time.sleep(0.5)

        setup_combat_state(driver)
        # Autopilot interceptor heading up
        add_interceptor(driver, 300, 400, armed=True, auto_pilot=True)
        # Target to the right — autopilot should steer toward it
        add_stationary_shahed(driver, 450, 300)
        # Fire a second (non-autopilot) interceptor to become active
        add_interceptor(driver, 500, 600, armed=False)

        step_frames(driver, 20)

        # First interceptor should have steered right (vx > 0)
        vx = driver.execute_script("return interceptors[0].alive ? interceptors[0].vx : null;")
        if vx is not None:
            assert vx > 0, f"Autopilot interceptor should steer right toward target, vx={vx}"
