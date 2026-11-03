// Ayuz - Unofficial Control Center for Asus Laptops
// Copyright (C) 2026 Guido Philipp
// SPDX-License-Identifier: GPL-3.0-or-later
//
// GNOME Shell counterpart of the KDE-only OLED care options in Ayuz. The Ayuz app
// writes the keys of org.gnome.shell.extensions.ayuz; this extension applies them.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// Same orbit as the ASUS OLED Shifter (and src/services/pixel_shift.rs), walked back and forth.
const ORBIT = [[0, 0], [1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0], [-1, -1], [0, -1], [1, -1]];
const DIM_EFFECT = 'ayuz-dim';
const SLIDE_MS = 200;
const HIDE_DELAY_MS = 500;

export default class AyuzExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._settingsIds = [
            'pixel-shift', 'pixel-shift-interval', 'panel-autohide',
            'panel-transparent', 'dim-inactive', 'dim-level',
        ].map(key => this._settings.connect(`changed::${key}`, () => this._sync()));

        this._shiftSource = 0;
        this._shiftInterval = 0;
        this._orbitIdx = 0;
        this._orbitForward = true;

        this._autohide = null;
        this._dimIds = null;

        this._sync();
    }

    disable() {
        for (const id of this._settingsIds)
            this._settings.disconnect(id);
        this._settingsIds = [];

        this._setShift(false, 0);
        this._setTransparent(false);
        this._setAutohide(false);
        this._setDim(false, 0);
        this._settings = null;
    }

    _sync() {
        const s = this._settings;
        this._setShift(s.get_boolean('pixel-shift'), s.get_uint('pixel-shift-interval'));
        this._setTransparent(s.get_boolean('panel-transparent'));
        this._setAutohide(s.get_boolean('panel-autohide'));
        this._setDim(s.get_boolean('dim-inactive'), s.get_double('dim-level'));
    }

    // ── Pixel shift ──────────────────────────────────────────────────────────

    _setShift(on, interval) {
        if (on && this._shiftSource && interval === this._shiftInterval)
            return;
        if (this._shiftSource) {
            GLib.source_remove(this._shiftSource);
            this._shiftSource = 0;
        }
        if (!on) {
            this._orbitIdx = 0;
            this._orbitForward = true;
            Main.panel.translation_x = 0;
            Main.panel.translation_y = 0;
            return;
        }
        this._shiftInterval = interval;
        this._shiftSource = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, interval, () => {
            this._stepOrbit();
            return GLib.SOURCE_CONTINUE;
        });
    }

    _stepOrbit() {
        if (this._orbitIdx === ORBIT.length - 1)
            this._orbitForward = false;
        else if (this._orbitIdx === 0)
            this._orbitForward = true;
        this._orbitIdx += this._orbitForward ? 1 : -1;
        const [dx, dy] = ORBIT[this._orbitIdx];
        Main.panel.translation_x = dx;
        Main.panel.translation_y = dy;
    }

    // ── Transparency ─────────────────────────────────────────────────────────

    _setTransparent(on) {
        if (on)
            Main.panel.add_style_class_name('ayuz-transparent');
        else
            Main.panel.remove_style_class_name('ayuz-transparent');
    }

    // ── Auto-hide ────────────────────────────────────────────────────────────

    _setAutohide(on) {
        if (on === (this._autohide !== null))
            return;
        const layout = Main.layoutManager;
        const box = layout.panelBox;

        if (on) {
            // Stop reserving screen space for the bar so windows can use the full height.
            layout.untrackChrome(box);
            layout.trackChrome(box, {affectsInputRegion: true, affectsStruts: false, trackFullscreen: true});

            const monitor = layout.primaryMonitor;
            const zone = new St.Widget({
                reactive: true,
                x: monitor.x,
                y: monitor.y,
                width: monitor.width,
                height: 1,
            });
            layout.addChrome(zone, {affectsInputRegion: true, trackFullscreen: true});

            this._autohide = {
                zone,
                hideSource: 0,
                ids: [
                    [zone, zone.connect('enter-event', () => this._showBar())],
                    [box, box.connect('leave-event', () => this._scheduleHide())],
                    [Main.overview, Main.overview.connect('showing', () => this._showBar())],
                    [Main.overview, Main.overview.connect('hiding', () => this._scheduleHide())],
                ],
            };
            this._hideBar();
        } else {
            const state = this._autohide;
            this._autohide = null;
            if (state.hideSource)
                GLib.source_remove(state.hideSource);
            for (const [obj, id] of state.ids)
                obj.disconnect(id);
            layout.removeChrome(state.zone);
            state.zone.destroy();

            box.remove_all_transitions();
            box.translation_y = 0;
            layout.untrackChrome(box);
            layout.trackChrome(box, {affectsInputRegion: true, affectsStruts: true, trackFullscreen: true});
        }
    }

    _showBar() {
        const state = this._autohide;
        if (!state)
            return;
        if (state.hideSource) {
            GLib.source_remove(state.hideSource);
            state.hideSource = 0;
        }
        Main.layoutManager.panelBox.ease({
            translation_y: 0,
            duration: SLIDE_MS,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        });
    }

    _scheduleHide() {
        const state = this._autohide;
        if (!state || state.hideSource)
            return;
        state.hideSource = GLib.timeout_add(GLib.PRIORITY_DEFAULT, HIDE_DELAY_MS, () => {
            state.hideSource = 0;
            this._hideBar();
            return GLib.SOURCE_REMOVE;
        });
    }

    _hideBar() {
        if (!this._autohide)
            return;
        const box = Main.layoutManager.panelBox;
        // Keep the bar while the overview or one of its menus is open, or the pointer is on it.
        if (Main.overview.visible || Main.panel.menuManager.activeMenu)
            return;
        const [, pointerY] = global.get_pointer();
        if (pointerY < box.y + box.height)
            return;
        box.ease({
            translation_y: -box.height,
            duration: SLIDE_MS,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        });
    }

    // ── Dim inactive windows ─────────────────────────────────────────────────

    _setDim(on, level) {
        if (!on) {
            if (this._dimIds) {
                for (const [obj, id] of this._dimIds)
                    obj.disconnect(id);
                this._dimIds = null;
            }
            for (const actor of global.get_window_actors())
                actor.remove_effect_by_name(DIM_EFFECT);
            return;
        }
        this._dimLevel = level;
        if (!this._dimIds) {
            const display = global.display;
            this._dimIds = [
                [display, display.connect('notify::focus-window', () => this._updateDim())],
                [display, display.connect('window-created', () => this._updateDim())],
            ];
        }
        this._updateDim();
    }

    _updateDim() {
        const focus = global.display.focus_window;
        for (const actor of global.get_window_actors()) {
            const win = actor.get_meta_window();
            const dim = win && win !== focus &&
                win.get_window_type() === Meta.WindowType.NORMAL;
            let effect = actor.get_effect(DIM_EFFECT);
            if (!dim) {
                if (effect)
                    actor.remove_effect(effect);
                continue;
            }
            if (!effect) {
                effect = new Clutter.BrightnessContrastEffect();
                actor.add_effect_with_name(DIM_EFFECT, effect);
            }
            effect.set_brightness(-this._dimLevel);
        }
    }
}
