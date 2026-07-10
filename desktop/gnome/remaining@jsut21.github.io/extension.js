import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const REFRESH_SECONDS = 300;
const PROVIDERS_PER_ROW = 3;
const PROVIDER_OPTIONS = [
    {id: 'codex', name: 'Codex', detail: 'ChatGPT OAuth'},
    {id: 'zai', name: 'Z.ai', detail: 'API key'},
    {id: 'ollama', name: 'Ollama Cloud', detail: 'Session cookie'},
    {id: 'opencode-go', name: 'OpenCode Go', detail: 'Workspace or local database'},
    {id: 'minimax', name: 'MiniMax', detail: 'API key or session cookie'},
];
const PROVIDER_ORDER = PROVIDER_OPTIONS.map(provider => provider.id);
const DEFAULT_PROVIDER_IDS = ['codex', 'zai', 'ollama'];

function findRemainingBinary() {
    const fromPath = GLib.find_program_in_path('remaining');
    if (fromPath)
        return fromPath;

    const home = GLib.get_home_dir();
    const candidates = [
        GLib.build_filenamev([home, '.local', 'bin', 'remaining']),
        GLib.build_filenamev([home, '.cargo', 'bin', 'remaining']),
        '/usr/local/bin/remaining',
        '/usr/bin/remaining',
    ];
    return candidates.find(path => GLib.file_test(path, GLib.FileTest.IS_EXECUTABLE)) ?? null;
}

function textLabel(text, styleClass, options = {}) {
    const label = new St.Label({
        text: String(text ?? ''),
        style_class: styleClass,
        x_expand: options.x_expand ?? false,
        y_align: options.y_align ?? Clutter.ActorAlign.CENTER,
    });

    if (options.wrap) {
        label.clutter_text.set_line_wrap(true);
        label.clutter_text.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR);
        label.clutter_text.set_ellipsize(Pango.EllipsizeMode.NONE);
    } else {
        label.clutter_text.set_ellipsize(Pango.EllipsizeMode.END);
    }

    return label;
}

function clamp(value, min, max) {
    return Math.max(min, Math.min(max, value));
}

function progressFillWidth(width, percent) {
    const available = Math.max(0, Number(width) || 0);
    const ratio = clamp(percent ?? 0, 0, 100) / 100;
    return Math.round(available * ratio);
}

function percentText(value) {
    if (value === null || value === undefined || Number.isNaN(value))
        return 'N/A';
    return `${Math.round(clamp(value, 0, 100))}%`;
}

function compactNumber(value, decimals = 1) {
    const number = Number(value);
    if (!Number.isFinite(number))
        return 'N/A';

    const abs = Math.abs(number);
    if (abs >= 1000000000)
        return `${(number / 1000000000).toFixed(decimals).replace(/\.0$/, '')}B`;
    if (abs >= 1000000)
        return `${(number / 1000000).toFixed(decimals).replace(/\.0$/, '')}M`;
    if (abs >= 1000)
        return `${(number / 1000).toFixed(decimals).replace(/\.0$/, '')}K`;
    if (Math.abs(number % 1) > 0.001)
        return number.toFixed(2).replace(/\.00$/, '');
    return Math.round(number).toLocaleString();
}

function unitLabel(unit) {
    switch (unit) {
    case 'tokens':
        return 'tokens';
    case 'credits':
        return 'credits';
    case 'requests':
        return 'requests';
    case 'minutes':
        return 'min';
    case 'units':
        return 'units';
    default:
        return '';
    }
}

function formatQuantity(value, unit) {
    if (value === null || value === undefined)
        return 'N/A';
    if (unit === 'usd')
        return `$${Number(value).toFixed(2)}`;
    if (unit === 'percent')
        return percentText(value);
    const suffix = unitLabel(unit);
    return suffix ? `${compactNumber(value)} ${suffix}` : compactNumber(value);
}

function remainingPercent(window) {
    if (!window)
        return null;
    if (typeof window.remaining_percent === 'number')
        return clamp(window.remaining_percent, 0, 100);
    if (typeof window.limit === 'number' && window.limit <= 0)
        return null;
    if (typeof window.remaining === 'number')
        return clamp(window.remaining, 0, 100);
    if (typeof window.used_percent === 'number')
        return clamp(100 - window.used_percent, 0, 100);
    return null;
}

function windowLabelParts(window) {
    const raw = String(window?.label ?? window?.type ?? 'Window').trim();
    const match = raw.match(/^(.*?)\s*\(([^)]+)\)\s*$/);
    if (!match)
        return {label: raw, group: null};

    return {
        label: match[1].trim() || raw,
        group: match[2].trim() || null,
    };
}

function windowGroupLabel(group) {
    const gpt = group?.match(/^GPT-([0-9.]+)-(.+)$/i);
    if (gpt)
        return `GPT-${gpt[1]} ${gpt[2].replace(/-/g, ' ')}`;
    return String(group ?? '').replace(/-/g, ' ');
}

function groupedWindows(windows) {
    const groups = [];
    for (const window of windows) {
        const parts = windowLabelParts(window);
        let group = groups.find(item => item.key === parts.group);
        if (!group) {
            group = {key: parts.group, windows: []};
            groups.push(group);
        }
        group.windows.push({window, label: parts.label});
    }
    return groups;
}

function toneForRemaining(value) {
    if (value === null || value === undefined || Number.isNaN(value))
        return 'unknown';
    if (value <= 15)
        return 'critical';
    if (value <= 35)
        return 'warning';
    return 'good';
}

function colorForRemaining(value) {
    switch (toneForRemaining(value)) {
    case 'critical':
        return [0.73, 0.36, 0.36];
    case 'warning':
        return [0.71, 0.48, 0.24];
    case 'good':
        return [0.37, 0.58, 0.44];
    default:
        return [0.52, 0.52, 0.52];
    }
}

function resetText(window) {
    if (!window || !window.resets_at_epoch)
        return 'No reset time';
    const now = Math.floor(Date.now() / 1000);
    const seconds = Math.max(0, Math.floor(window.resets_at_epoch - now));
    if (seconds <= 0)
        return 'Resets now';
    const minutes = Math.max(1, Math.floor(seconds / 60));
    const days = Math.floor(minutes / 1440);
    const hours = Math.floor((minutes % 1440) / 60);
    const mins = minutes % 60;
    if (days > 0)
        return `Resets in ${days}d ${hours}h`;
    if (hours > 0)
        return `Resets in ${hours}h ${mins}m`;
    return `Resets in ${mins}m`;
}

function localDateTimeText(epoch) {
    if (!epoch)
        return null;
    const date = new Date(epoch * 1000);
    const pad = value => String(value).padStart(2, '0');
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function expiryText(epoch) {
    const localTime = localDateTimeText(epoch);
    if (!localTime)
        return null;
    const now = Math.floor(Date.now() / 1000);
    const seconds = Math.max(0, Math.floor(epoch - now));
    const days = Math.ceil(seconds / 86400);
    if (seconds <= 0)
        return `Expires now · ${localTime}`;
    if (days <= 1)
        return `Expires today · ${localTime}`;
    return `Expires in ${days}d · ${localTime}`;
}

function statusText(status) {
    switch (status) {
    case 'available':
        return 'Available';
    case 'auth_required':
        return 'Auth required';
    case 'network_error':
        return 'Network error';
    case 'unavailable':
        return 'Unavailable';
    default:
        return status ? String(status) : 'Unknown';
    }
}

function sourceText(provider) {
    const source = provider?.source ? String(provider.source).replace(/_/g, ' ') : null;
    const sourceLabel = provider?.source_label;
    if (source && sourceLabel)
        return `${source} · ${sourceLabel}`;
    return source ?? sourceLabel ?? 'No source';
}

function windowAmountText(window) {
    const unit = window?.unit ?? 'percent';
    if (unit !== 'percent') {
        const remaining = window.remaining_units ?? window.remaining;
        const limit = window.limit_units ?? window.limit;
        if (remaining !== null && remaining !== undefined && limit !== null && limit !== undefined && Number(limit) > 0)
            return `${formatQuantity(remaining, unit)} left / ${formatQuantity(limit, unit)}`;
        if (remaining !== null && remaining !== undefined)
            return `${formatQuantity(remaining, unit)} left`;
    }

    if (typeof window?.used_percent === 'number')
        return `Used ${percentText(window.used_percent)}`;
    return null;
}

function creditValueText(credit) {
    const unit = credit?.unit ?? 'units';
    if (unit === 'credits' && credit?.remaining === 1 && credit?.total === 1)
        return 'Available';
    if (credit?.remaining !== null && credit?.remaining !== undefined && credit?.total !== null && credit?.total !== undefined)
        return `${formatQuantity(credit.remaining, unit)} / ${formatQuantity(credit.total, unit)}`;
    if (credit?.remaining !== null && credit?.remaining !== undefined)
        return formatQuantity(credit.remaining, unit);
    return 'N/A';
}

function metricValueText(metric) {
    return formatQuantity(metric?.value, metric?.unit ?? 'units');
}

function windowByType(provider, type) {
    return provider?.windows?.find(w => w.type === type) ?? null;
}

function providerDisplayName(provider) {
    if (!provider)
        return 'Usage';
    if (provider.provider === 'ollama')
        return 'Ollama Cloud';
    return provider.name ?? provider.provider ?? 'Usage';
}

function summaryText(provider) {
    if (!provider || provider.error)
        return `${providerDisplayName(provider)} ?`;
    const five = windowByType(provider, '5h');
    const weekly = windowByType(provider, 'weekly');
    const parts = [];
    if (five)
        parts.push(`5h ${percentText(remainingPercent(five))}`);
    if (weekly)
        parts.push(`W ${percentText(remainingPercent(weekly))}`);
    if (parts.length > 0)
        return `${providerDisplayName(provider)} ${parts.join(' | ')}`;
    const first = provider.windows?.[0];
    return `${providerDisplayName(provider)} ${percentText(remainingPercent(first))}`;
}

function activeProvider(data, providerId = null) {
    return data.find(provider => provider.provider === providerId) ??
        data.find(provider => provider.provider === 'codex') ?? data[0] ?? null;
}

function orderedProviders(data) {
    return [...data].sort((left, right) => {
        const leftIndex = PROVIDER_ORDER.indexOf(left.provider);
        const rightIndex = PROVIDER_ORDER.indexOf(right.provider);
        if (leftIndex >= 0 || rightIndex >= 0) {
            if (leftIndex < 0)
                return 1;
            if (rightIndex < 0)
                return -1;
            return leftIndex - rightIndex;
        }
        return providerDisplayName(left).localeCompare(providerDisplayName(right));
    });
}

function providerHealthWindow(provider) {
    const known = (provider?.windows ?? [])
        .filter(window => remainingPercent(window) !== null)
        .sort((left, right) => remainingPercent(left) - remainingPercent(right));
    return known[0] ?? provider?.windows?.[0] ?? null;
}

const Meter = GObject.registerClass(
class Meter extends St.DrawingArea {
    _init() {
        super._init({style_class: 'remaining-panel-meter'});
        this.set_size(18, 18);
        this._primary = null;
        this._secondary = null;
        this.connect('repaint', this._draw.bind(this));
    }

    setPercents(primary, secondary) {
        this._primary = primary;
        this._secondary = secondary;
        this.queue_repaint();
    }

    _arc(cr, cx, cy, radius, percent, width) {
        cr.setLineWidth(width);
        cr.setLineCap(1);
        cr.setSourceRGBA(1, 1, 1, 0.20);
        cr.arc(cx, cy, radius, 0, Math.PI * 2);
        cr.stroke();
        const [r, g, b] = colorForRemaining(percent);
        cr.setSourceRGBA(r, g, b, 1);
        cr.arc(cx, cy, radius, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * clamp(percent ?? 0, 0, 100) / 100);
        cr.stroke();
    }

    _draw(area) {
        const cr = area.get_context();
        const [width, height] = area.get_surface_size();
        const cx = width / 2;
        const cy = height / 2;
        this._arc(cr, cx, cy, 7.1, this._primary, 2.4);
        if (this._secondary !== null && this._secondary !== undefined)
            this._arc(cr, cx, cy, 4.1, this._secondary, 2.0);
        const [r, g, b] = colorForRemaining(this._secondary ?? this._primary);
        cr.setSourceRGBA(r, g, b, 1);
        cr.arc(cx, cy, 1.7, 0, Math.PI * 2);
        cr.fill();
        cr.$dispose();
    }
});

const ProgressBar = GObject.registerClass(
class ProgressBar extends St.Bin {
    _init(percent, compact = false) {
        super._init({
            style_class: compact
                ? 'remaining-progress-track remaining-progress-track-compact'
                : 'remaining-progress-track',
            x_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            clip_to_allocation: true,
        });
        this._percent = clamp(percent ?? 0, 0, 100);
        this.set_height(compact ? 4 : 7);

        this._fill = new St.Widget({
            style_class: [
                'remaining-progress-fill',
                compact ? 'remaining-progress-fill-compact' : '',
                'remaining-fill-' + toneForRemaining(this._percent),
            ].filter(Boolean).join(' '),
        });
        this.set_child(this._fill);
    }

    vfunc_allocate(box) {
        const contentBox = this.get_theme_node().get_content_box(box);
        this.set_allocation(box);

        if (!this.child)
            return;

        const fillWidth = progressFillWidth(
            contentBox.x2 - contentBox.x1,
            this._percent
        );
        contentBox.x2 = contentBox.x1 + fillWidth;
        this.child.allocate(contentBox);
    }
});

const RemainingIndicator = GObject.registerClass(
class RemainingIndicator extends PanelMenu.Button {
    _init(settings) {
        super._init(0.0, 'Remaining Bar');
        this._settings = settings;
        this._data = [];
        this._expandedSections = new Set();
        this._initialRefreshId = 0;
        this._timeoutId = 0;
        this._watchdogId = 0;
        this._menuSignalId = 0;
        this._menuDirty = false;
        this._refreshing = false;
        this._disposed = false;
        this._proc = null;
        this._cancellable = null;
        this._lastError = null;
        this._showSettings = false;
        this._remaining = findRemainingBinary();
        this._enabledProviderIds = this._readEnabledProviders();
        this._allowBrowserCookies = this._getBooleanSetting('allow-browser-cookies');
        this._allowCustomEndpoints = this._getBooleanSetting('allow-custom-endpoints');
        this._panelProviderId = this._settings?.get_string('panel-provider') || 'codex';
        this._selectedProviderId = this._panelProviderId;

        const panelBox = new St.BoxLayout({
            style_class: 'remaining-panel-button',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._meter = new Meter();
        this._label = new St.Label({
            text: 'Usage ...',
            style_class: 'remaining-panel-label',
            y_align: Clutter.ActorAlign.CENTER,
        });
        panelBox.add_child(this._meter);
        panelBox.add_child(this._label);
        this.add_child(panelBox);

        this._rootItem = new PopupMenu.PopupBaseMenuItem({
            reactive: false,
            can_focus: false,
        });
        this._root = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-menu-root',
        });
        this._rootItem.add_child(this._root);
        this.menu.addMenuItem(this._rootItem);
        this._menuSignalId = this.menu.connect(
            'open-state-changed',
            (_menu, isOpen) => {
                if (isOpen && this._menuDirty)
                    this._renderMenu();
            }
        );

        this._renderMenu();
        this._initialRefreshId = GLib.idle_add(
            GLib.PRIORITY_DEFAULT_IDLE,
            () => {
                this._initialRefreshId = 0;
                this.refresh();
                return GLib.SOURCE_REMOVE;
            }
        );
        this._timeoutId = GLib.timeout_add_seconds(
            GLib.PRIORITY_DEFAULT_IDLE,
            REFRESH_SECONDS,
            () => {
                this.refresh();
                return GLib.SOURCE_CONTINUE;
            }
        );
    }

    destroy() {
        this._disposed = true;
        if (this._initialRefreshId) {
            GLib.Source.remove(this._initialRefreshId);
            this._initialRefreshId = 0;
        }
        if (this._timeoutId) {
            GLib.Source.remove(this._timeoutId);
            this._timeoutId = 0;
        }
        this._clearRefreshWatchdog();
        if (this._menuSignalId) {
            this.menu.disconnect(this._menuSignalId);
            this._menuSignalId = 0;
        }
        this._cancellable?.cancel();
        this._cancellable = null;
        try {
            this._proc?.force_exit();
        } catch {
            // The process may already have exited.
        }
        this._proc = null;
        super.destroy();
    }

    _clearRefreshWatchdog() {
        if (!this._watchdogId)
            return;
        GLib.Source.remove(this._watchdogId);
        this._watchdogId = 0;
    }

    _startRefreshWatchdog(proc, cancellable) {
        this._clearRefreshWatchdog();
        this._watchdogId = GLib.timeout_add_seconds(
            GLib.PRIORITY_DEFAULT_IDLE,
            30,
            () => {
                this._watchdogId = 0;
                if (this._disposed || this._proc !== proc)
                    return GLib.SOURCE_REMOVE;

                cancellable.cancel();
                try {
                    proc.force_exit();
                } catch {
                    // The process may have exited while the watchdog fired.
                }
                this._proc = null;
                this._cancellable = null;
                this._finishRefreshError('Update timed out');
                return GLib.SOURCE_REMOVE;
            }
        );
    }

    _readEnabledProviders() {
        try {
            const configured = this._settings?.get_strv('enabled-providers') ?? [];
            const supported = configured.filter(id => PROVIDER_ORDER.includes(id));
            return supported.length > 0 ? [...new Set(supported)] : [...DEFAULT_PROVIDER_IDS];
        } catch (error) {
            console.warn('Remaining could not read enabled providers: ' + error.message);
            return [...DEFAULT_PROVIDER_IDS];
        }
    }

    _getBooleanSetting(key) {
        try {
            return this._settings?.get_boolean(key) ?? false;
        } catch (error) {
            console.warn('Remaining could not read ' + key + ': ' + error.message);
            return false;
        }
    }

    _commandArgs() {
        const args = [
            this._remaining,
            '--json',
            '--no-color',
            '--provider',
            this._enabledProviderIds.join(','),
        ];
        if (this._allowBrowserCookies)
            args.push('--browser-cookies');
        else
            args.push('--no-browser-cookies');
        if (this._allowCustomEndpoints)
            args.push('--allow-custom-endpoints');
        return args;
    }

    refresh() {
        if (this._disposed || this._refreshing)
            return;

        if (!this._remaining) {
            this._lastError = 'remaining executable not found';
            this._label.set_text('Usage ?');
            this._meter.setPercents(null, null);
            this._requestMenuRender();
            return;
        }

        this._refreshing = true;
        this._lastError = null;
        if (this._data.length === 0)
            this._label.set_text('Usage ...');
        this._requestMenuRender();

        let proc;
        try {
            proc = Gio.Subprocess.new(
                this._commandArgs(),
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE
            );
        } catch (error) {
            this._finishRefreshError(error.message);
            return;
        }

        const cancellable = new Gio.Cancellable();
        this._proc = proc;
        this._cancellable = cancellable;
        this._startRefreshWatchdog(proc, cancellable);
        proc.communicate_utf8_async(null, cancellable, (subprocess, res) => {
            try {
                const [, stdout, stderr] = subprocess.communicate_utf8_finish(res);
                if (this._disposed || cancellable.is_cancelled())
                    return;
                if (!subprocess.get_successful())
                    throw new Error(stderr || 'remaining failed');

                const parsed = JSON.parse(stdout);
                if (!Array.isArray(parsed))
                    throw new Error('remaining returned invalid JSON');

                this._data = orderedProviders(parsed);
                this._ensureProviderSelection();
                this._refreshing = false;
                this._lastError = null;
                this._renderPanel();
                this._requestMenuRender();
            } catch (error) {
                if (this._disposed || cancellable.is_cancelled())
                    return;
                this._finishRefreshError(error.message);
            } finally {
                if (!this._disposed && this._cancellable === cancellable) {
                    this._clearRefreshWatchdog();
                    this._proc = null;
                    this._cancellable = null;
                }
            }
        });
    }

    _finishRefreshError(message) {
        this._refreshing = false;
        this._lastError = String(message || 'Unknown update error').trim();
        if (this._data.length === 0) {
            this._label.set_text('Usage ?');
            this._meter.setPercents(null, null);
        } else {
            this._renderPanel();
        }
        this._requestMenuRender();
    }

    _ensureProviderSelection() {
        if (this._data.length === 0)
            return;

        const ids = new Set(this._data.map(provider => provider.provider));
        if (!ids.has(this._panelProviderId)) {
            this._panelProviderId = activeProvider(this._data)?.provider;
            this._storePanelProvider();
        }
        if (!ids.has(this._selectedProviderId))
            this._selectedProviderId = this._panelProviderId;
    }

    _storePanelProvider() {
        if (!this._settings || !this._panelProviderId)
            return;
        try {
            if (this._settings.get_string('panel-provider') !== this._panelProviderId)
                this._settings.set_string('panel-provider', this._panelProviderId);
        } catch (error) {
            console.warn('Remaining Bar could not save panel provider: ' + error.message);
        }
    }

    _restartRefresh() {
        this._clearRefreshWatchdog();
        this._cancellable?.cancel();
        this._cancellable = null;
        try {
            this._proc?.force_exit();
        } catch {
            // The process may already have exited.
        }
        this._proc = null;
        this._refreshing = false;
        this._lastError = null;
        this.refresh();
    }

    _setProviderEnabled(providerId, enabled) {
        const current = new Set(this._enabledProviderIds);
        if (enabled)
            current.add(providerId);
        else if (current.size === 1)
            return false;
        else
            current.delete(providerId);

        const nextEnabledProviderIds = PROVIDER_ORDER.filter(id => current.has(id));
        try {
            if (this._settings?.set_strv('enabled-providers', nextEnabledProviderIds) === false)
                return false;
        } catch (error) {
            console.warn('Remaining could not save enabled providers: ' + error.message);
            return false;
        }
        this._enabledProviderIds = nextEnabledProviderIds;

        const enabledIds = new Set(this._enabledProviderIds);
        this._data = this._data.filter(provider => enabledIds.has(provider.provider));
        this._expandedSections.clear();
        if (!enabledIds.has(this._panelProviderId)) {
            this._panelProviderId = this._enabledProviderIds[0];
            this._storePanelProvider();
        }
        if (!enabledIds.has(this._selectedProviderId))
            this._selectedProviderId = this._panelProviderId;
        this._renderPanel();
        this._restartRefresh();
        return true;
    }

    _setDataAccess(key, value) {
        try {
            if (this._settings?.set_boolean(key, value) === false)
                return false;
        } catch (error) {
            console.warn('Remaining could not save ' + key + ': ' + error.message);
            return false;
        }
        if (key === 'allow-browser-cookies')
            this._allowBrowserCookies = value;
        else if (key === 'allow-custom-endpoints')
            this._allowCustomEndpoints = value;
        this._restartRefresh();
        return true;
    }

    _toggleSettings() {
        this._showSettings = !this._showSettings;
        this._renderMenu();
    }

    _selectProvider(providerId) {
        if (providerId === this._selectedProviderId)
            return;
        this._selectedProviderId = providerId;
        this._renderMenu();
    }

    _pinProvider(providerId) {
        if (providerId === this._panelProviderId)
            return;
        this._panelProviderId = providerId;
        this._storePanelProvider();
        this._renderPanel();
        this._renderMenu();
    }

    _toggleSection(key) {
        if (this._expandedSections.has(key)) {
            this._expandedSections.delete(key);
        } else {
            const providerPrefix = key.split(':')[0] + ':';
            for (const expanded of [...this._expandedSections]) {
                if (expanded.startsWith(providerPrefix))
                    this._expandedSections.delete(expanded);
            }
            this._expandedSections.add(key);
        }
        this._renderMenu();
    }

    _clearRoot() {
        for (const child of this._root.get_children())
            child.destroy();
    }

    _renderPanel() {
        const active = activeProvider(this._data, this._panelProviderId);
        if (!active) {
            this._label.set_text('Usage unavailable');
            this._meter.setPercents(null, null);
            return;
        }

        this._label.set_text(summaryText(active));
        const five = windowByType(active, '5h');
        const weekly = windowByType(active, 'weekly');
        this._meter.setPercents(
            remainingPercent(five ?? active.windows?.[0]),
            remainingPercent(weekly)
        );
    }

    _headerSubtitle() {
        if (this._refreshing)
            return this._data.length > 0
                ? 'Updating providers... · showing previous data'
                : 'Updating providers...';
        if (this._lastError)
            return this._data.length > 0
                ? 'Update failed · showing previous data'
                : 'Update failed';

        const fetchedAt = this._data[0]?.fetched_at;
        if (!fetchedAt)
            return 'Waiting for usage data';
        const time = fetchedAt.split('T')[1]?.slice(0, 8) ?? '';
        return 'Updated ' + time + ' · ' + this._data.length + ' providers';
    }

    _requestMenuRender() {
        this._menuDirty = true;
        if (!this._disposed && this.menu.isOpen)
            this._renderMenu();
    }

    _renderMenu() {
        if (this._disposed)
            return;
        this._menuDirty = false;
        this._clearRoot();
        this._root.add_child(this._header(
            this._showSettings ? 'Settings' : 'Remaining',
            this._showSettings
                ? this._enabledProviderIds.length + ' providers enabled'
                : this._headerSubtitle(),
            !this._showSettings,
            !this._showSettings && Boolean(this._lastError)
        ));

        if (this._showSettings) {
            this._root.add_child(this._settingsView());
            return;
        }

        if (this._data.length === 0) {
            const message = new St.BoxLayout({
                vertical: true,
                style_class: 'remaining-message-card',
            });
            const title = this._lastError ? 'Usage data unavailable' : 'No provider data';
            message.add_child(textLabel(title, 'remaining-message-title'));
            message.add_child(textLabel(
                this._lastError ?? 'No configured provider returned usage data.',
                this._lastError ? 'remaining-error' : 'remaining-empty',
                {wrap: true, x_expand: true, y_align: Clutter.ActorAlign.START}
            ));
            this._root.add_child(message);
            return;
        }

        this._ensureProviderSelection();
        const scroll = new St.ScrollView({
            style_class: 'remaining-menu-scroll',
            overlay_scrollbars: true,
            x_expand: true,
        });
        scroll.set_policy(St.PolicyType.NEVER, St.PolicyType.AUTOMATIC);

        const content = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-menu-content',
            x_expand: true,
        });
        content.add_child(this._providerSelector());
        content.add_child(new St.Widget({
            style_class: 'remaining-divider',
            x_expand: true,
        }));

        const selected = activeProvider(this._data, this._selectedProviderId);
        if (selected)
            content.add_child(this._providerDetail(selected));

        scroll.set_child(content);
        this._root.add_child(scroll);
    }

    _header(title, subtitle, includeRefresh = false, isError = false) {
        const row = new St.BoxLayout({
            style_class: 'remaining-header-row',
            x_expand: true,
        });
        const text = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-header-text',
            x_expand: true,
        });
        text.add_child(textLabel(title, 'remaining-header-title'));
        text.add_child(textLabel(
            subtitle,
            isError
                ? 'remaining-header-subtitle remaining-header-subtitle-error'
                : 'remaining-header-subtitle',
            {x_expand: true}
        ));
        row.add_child(text);

        const actions = new St.BoxLayout({
            style_class: 'remaining-header-actions',
        });
        const settings = new St.Button({
            style_class: 'remaining-icon-button',
            can_focus: true,
            reactive: true,
            track_hover: true,
            accessible_name: this._showSettings ? 'Back to usage' : 'Open settings',
        });
        settings.set_child(new St.Icon({
            icon_name: this._showSettings
                ? 'go-previous-symbolic'
                : 'preferences-system-symbolic',
            style_class: 'remaining-icon',
        }));
        settings.connect('clicked', () => this._toggleSettings());
        actions.add_child(settings);

        if (includeRefresh) {
            const styleClass = this._refreshing
                ? 'remaining-icon-button remaining-icon-button-loading'
                : 'remaining-icon-button';
            const refresh = new St.Button({
                style_class: styleClass,
                can_focus: !this._refreshing,
                reactive: !this._refreshing,
                track_hover: !this._refreshing,
                accessible_name: this._refreshing ? 'Updating usage' : 'Refresh usage',
            });
            refresh.set_child(new St.Icon({
                icon_name: this._refreshing
                    ? 'process-working-symbolic'
                    : 'view-refresh-symbolic',
                style_class: 'remaining-icon',
            }));
            refresh.connect('clicked', () => this.refresh());
            actions.add_child(refresh);
        }
        row.add_child(actions);
        return row;
    }

    _settingsView() {
        const scroll = new St.ScrollView({
            style_class: 'remaining-menu-scroll remaining-settings-scroll',
            overlay_scrollbars: true,
            x_expand: true,
        });
        scroll.set_policy(St.PolicyType.NEVER, St.PolicyType.AUTOMATIC);

        const content = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-settings-content',
            x_expand: true,
        });
        content.add_child(textLabel('Providers', 'remaining-settings-section-title'));

        const providers = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-settings-list',
            x_expand: true,
        });
        for (const provider of PROVIDER_OPTIONS) {
            providers.add_child(this._settingsToggleRow({
                title: provider.name,
                subtitle: provider.detail,
                active: this._enabledProviderIds.includes(provider.id),
                onToggle: value => this._setProviderEnabled(provider.id, value),
            }));
        }
        content.add_child(providers);
        content.add_child(new St.Widget({
            style_class: 'remaining-divider remaining-settings-divider',
            x_expand: true,
        }));
        content.add_child(textLabel('Data access', 'remaining-settings-section-title'));

        const access = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-settings-list',
            x_expand: true,
        });
        access.add_child(this._settingsToggleRow({
            title: 'Browser cookies',
            subtitle: 'Read supported cookies from local browser profiles',
            active: this._allowBrowserCookies,
            onToggle: value => this._setDataAccess('allow-browser-cookies', value),
        }));
        access.add_child(this._settingsToggleRow({
            title: 'Custom endpoints',
            subtitle: 'Allow credentials on non-official HTTPS endpoints',
            active: this._allowCustomEndpoints,
            onToggle: value => this._setDataAccess('allow-custom-endpoints', value),
        }));
        content.add_child(access);

        scroll.set_child(content);
        return scroll;
    }

    _settingsToggleRow({title, subtitle, active, onToggle}) {
        const button = new St.Button({
            style_class: 'remaining-settings-row',
            can_focus: true,
            track_hover: true,
            x_expand: true,
        });
        const content = new St.BoxLayout({
            style_class: 'remaining-settings-row-content',
            x_expand: true,
        });
        const copy = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-settings-row-copy',
            x_expand: true,
        });
        copy.add_child(textLabel(title, 'remaining-settings-row-title', {x_expand: true}));
        copy.add_child(textLabel(subtitle, 'remaining-settings-row-subtitle', {x_expand: true}));
        content.add_child(copy);

        const toggle = new PopupMenu.Switch(active);
        content.add_child(toggle);
        button.set_child(content);
        button.connect('clicked', () => {
            toggle.toggle();
            if (onToggle(toggle.state) === false)
                toggle.toggle();
        });
        return button;
    }

    _providerSelector() {
        const section = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-provider-selector',
            x_expand: true,
        });
        const heading = new St.BoxLayout({
            style_class: 'remaining-selector-heading',
            x_expand: true,
        });
        heading.add_child(textLabel(
            'Panel display',
            'remaining-selector-title',
            {x_expand: true}
        ));

        const panelProvider = activeProvider(this._data, this._panelProviderId);
        const panelState = new St.BoxLayout({
            style_class: 'remaining-panel-provider-state',
        });
        panelState.add_child(new St.Icon({
            icon_name: 'view-pin-symbolic',
            style_class: 'remaining-panel-provider-icon',
        }));
        panelState.add_child(textLabel(
            providerDisplayName(panelProvider),
            'remaining-panel-provider-label'
        ));
        heading.add_child(panelState);
        section.add_child(heading);

        const providers = orderedProviders(this._data);
        const rows = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-provider-summary-grid',
            x_expand: true,
        });
        for (let index = 0; index < providers.length; index += PROVIDERS_PER_ROW) {
            const row = new St.BoxLayout({
                style_class: 'remaining-provider-summary-row',
                x_expand: true,
            });
            for (const provider of providers.slice(index, index + PROVIDERS_PER_ROW))
                row.add_child(this._providerSummaryCard(provider));
            rows.add_child(row);
        }
        section.add_child(rows);
        return section;
    }

    _providerSummaryCard(provider) {
        const selected = provider.provider === this._selectedProviderId;
        const pinned = provider.provider === this._panelProviderId;
        const classes = ['remaining-provider-summary-card'];
        if (selected)
            classes.push('remaining-provider-summary-card-selected');

        const card = new St.BoxLayout({
            vertical: true,
            style_class: classes.join(' '),
            x_expand: true,
        });

        const header = new St.BoxLayout({
            style_class: 'remaining-provider-summary-header',
            x_expand: true,
        });
        const identity = new St.BoxLayout({
            style_class: 'remaining-provider-summary-identity',
            x_expand: true,
        });
        const remaining = remainingPercent(providerHealthWindow(provider));
        const tone = toneForRemaining(remaining);
        identity.add_child(new St.Widget({
            style_class: 'remaining-provider-status-dot remaining-fill-' + tone,
        }));
        identity.add_child(textLabel(
            providerDisplayName(provider),
            'remaining-provider-summary-name',
            {x_expand: true}
        ));

        const selectHeader = new St.Button({
            style_class: 'remaining-provider-select-button',
            can_focus: true,
            track_hover: true,
            x_expand: true,
        });
        selectHeader.set_child(identity);
        selectHeader.connect('clicked', () =>
            this._selectProvider(provider.provider));
        header.add_child(selectHeader);

        const pin = new St.Button({
            style_class: pinned
                ? 'remaining-pin-button remaining-pin-button-active'
                : 'remaining-pin-button',
            can_focus: true,
            track_hover: true,
            accessible_name: 'Show ' + providerDisplayName(provider) + ' in the top panel',
        });
        pin.set_child(new St.Icon({
            icon_name: 'view-pin-symbolic',
            style_class: 'remaining-pin-icon',
        }));
        pin.connect('clicked', () => this._pinProvider(provider.provider));
        header.add_child(pin);
        card.add_child(header);

        const metricsButton = new St.Button({
            style_class: 'remaining-provider-metrics-button',
            can_focus: true,
            track_hover: true,
            x_expand: true,
        });
        const metrics = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-provider-summary-metrics',
            x_expand: true,
        });
        const summaryWindows = this._summaryWindows(provider);
        if (summaryWindows.length === 0) {
            metrics.add_child(textLabel(
                statusText(provider.status),
                'remaining-provider-summary-empty'
            ));
        } else {
            for (const window of summaryWindows)
                metrics.add_child(this._providerSummaryMetric(window));
        }
        metricsButton.set_child(metrics);
        metricsButton.connect('clicked', () =>
            this._selectProvider(provider.provider));
        card.add_child(metricsButton);
        return card;
    }

    _summaryWindows(provider) {
        const five = windowByType(provider, '5h');
        const weekly = windowByType(provider, 'weekly');
        const primary = [five, weekly].filter(Boolean);
        if (primary.length > 0)
            return primary;
        return (provider.windows ?? []).slice(0, 2);
    }

    _providerSummaryMetric(window) {
        const metric = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-provider-summary-metric',
            x_expand: true,
        });
        const top = new St.BoxLayout({
            style_class: 'remaining-provider-summary-values',
            x_expand: true,
        });
        const parts = windowLabelParts(window);
        top.add_child(textLabel(
            parts.label,
            'remaining-provider-summary-metric-label',
            {x_expand: true}
        ));
        const remaining = remainingPercent(window);
        top.add_child(textLabel(
            percentText(remaining),
            'remaining-provider-summary-metric-value remaining-text-' +
                toneForRemaining(remaining)
        ));
        metric.add_child(top);
        metric.add_child(new ProgressBar(remaining, true));
        return metric;
    }

    _providerDetail(provider) {
        const detail = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-provider-detail',
            x_expand: true,
        });
        detail.add_child(this._providerDetailHeader(provider));

        if (provider.error) {
            detail.add_child(textLabel(
                String(provider.error),
                'remaining-error',
                {wrap: true, x_expand: true, y_align: Clutter.ActorAlign.START}
            ));
            return detail;
        }

        const groups = groupedWindows(provider.windows ?? []);
        const primary = groups.find(group => !group.key);
        if (primary?.windows.length)
            detail.add_child(this._windowRows(primary.windows, false));

        const more = [];
        for (const group of groups.filter(item => item.key))
            more.push(this._windowGroupPanel(provider, group));
        if ((provider.credits?.length ?? 0) > 0)
            more.push(this._creditsPanel(provider));
        if ((provider.metrics?.length ?? 0) > 0)
            more.push(this._metricsPanel(provider));

        if (more.length > 0) {
            const sections = new St.BoxLayout({
                vertical: true,
                style_class: 'remaining-more-sections',
                x_expand: true,
            });
            sections.add_child(textLabel('More', 'remaining-more-title'));
            for (const item of more)
                sections.add_child(item);
            detail.add_child(sections);
        }

        if (!primary && more.length === 0)
            detail.add_child(textLabel('No quota details returned', 'remaining-empty'));
        return detail;
    }

    _providerDetailHeader(provider) {
        const row = new St.BoxLayout({
            style_class: 'remaining-provider-detail-header',
            x_expand: true,
        });
        const identity = new St.BoxLayout({
            style_class: 'remaining-provider-detail-identity',
            x_expand: true,
        });
        const tone = toneForRemaining(
            remainingPercent(providerHealthWindow(provider))
        );
        const mark = new St.BoxLayout({
            style_class: 'remaining-provider-detail-mark',
        });
        mark.add_child(new St.Widget({
            style_class: 'remaining-provider-detail-dot remaining-fill-' + tone,
        }));
        identity.add_child(mark);

        const copy = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-provider-detail-copy',
            x_expand: true,
        });
        copy.add_child(textLabel(
            providerDisplayName(provider),
            'remaining-provider-detail-title'
        ));
        copy.add_child(textLabel(
            sourceText(provider),
            'remaining-source-label',
            {x_expand: true}
        ));
        identity.add_child(copy);
        row.add_child(identity);

        const meta = new St.BoxLayout({
            style_class: 'remaining-provider-detail-meta',
        });
        meta.add_child(textLabel(
            statusText(provider.status),
            'remaining-status-pill'
        ));
        if (provider.plan)
            meta.add_child(textLabel(provider.plan, 'remaining-provider-plan'));
        row.add_child(meta);
        return row;
    }

    _windowRows(windows, compact = false) {
        const grid = new St.BoxLayout({
            vertical: true,
            style_class: compact
                ? 'remaining-window-grid remaining-window-grid-compact'
                : 'remaining-window-grid',
            x_expand: true,
        });
        for (let index = 0; index < windows.length; index += 2) {
            const row = new St.BoxLayout({
                style_class: 'remaining-window-row',
                x_expand: true,
            });
            row.add_child(this._windowTile(
                windows[index].window,
                windows[index].label,
                compact
            ));
            if (windows[index + 1]) {
                row.add_child(this._windowTile(
                    windows[index + 1].window,
                    windows[index + 1].label,
                    compact
                ));
            }
            grid.add_child(row);
        }
        return grid;
    }

    _windowTile(window, label, compact = false) {
        const remaining = remainingPercent(window);
        const tone = toneForRemaining(remaining);
        const tile = new St.BoxLayout({
            vertical: true,
            style_class: compact
                ? 'remaining-window-tile remaining-window-tile-compact'
                : 'remaining-window-tile',
            x_expand: true,
        });
        const top = new St.BoxLayout({
            style_class: 'remaining-window-top',
            x_expand: true,
        });
        top.add_child(textLabel(
            label ?? window.label ?? window.type ?? 'Window',
            'remaining-window-label',
            {x_expand: true}
        ));
        top.add_child(textLabel(
            percentText(remaining) + ' left',
            'remaining-window-percent remaining-text-' + tone
        ));
        tile.add_child(top);
        tile.add_child(new ProgressBar(remaining, compact));

        const amount = windowAmountText(window);
        if (amount) {
            tile.add_child(textLabel(
                amount,
                'remaining-window-detail',
                {x_expand: true}
            ));
        }
        tile.add_child(textLabel(
            resetText(window),
            'remaining-window-reset',
            {x_expand: true}
        ));
        return tile;
    }

    _windowGroupPanel(provider, group) {
        const key = [
            provider.provider,
            'windows',
            group.key,
        ].join(':');
        const summary = group.windows.map(item =>
            item.label + ' ' + percentText(remainingPercent(item.window))
        ).join(' · ');
        return this._collapsiblePanel({
            key,
            title: windowGroupLabel(group.key),
            summary,
            iconName: 'applications-system-symbolic',
            bodyFactory: () => this._windowRows(group.windows, true),
        });
    }

    _creditsPanel(provider) {
        const credits = [...(provider.credits ?? [])].sort((left, right) =>
            (left.expires_at_epoch ?? Number.MAX_SAFE_INTEGER) -
            (right.expires_at_epoch ?? Number.MAX_SAFE_INTEGER)
        );
        const available = credits.filter(credit =>
            Number(credit.remaining ?? 0) > 0
        ).length;
        const nextExpiry = expiryText(credits[0]?.expires_at_epoch);
        const summary = available + ' available' +
            (nextExpiry ? ' · ' + nextExpiry.toLowerCase() : '');

        return this._collapsiblePanel({
            key: provider.provider + ':credits',
            title: 'Full reset credits',
            summary,
            iconName: 'document-properties-symbolic',
            bodyFactory: () => {
                const list = new St.BoxLayout({
                    vertical: true,
                    style_class: 'remaining-detail-list',
                    x_expand: true,
                });
                for (const credit of credits) {
                    list.add_child(this._detailRow({
                        label: credit.label ?? 'Credit',
                        value: creditValueText(credit),
                        meta: [credit.source, expiryText(credit.expires_at_epoch)]
                            .filter(Boolean)
                            .join(' · '),
                    }));
                }
                return list;
            },
        });
    }

    _metricsPanel(provider) {
        const metrics = provider.metrics ?? [];
        const values = metrics
            .map(metric => Number(metric.value))
            .filter(Number.isFinite);
        const total = values.reduce((sum, value) => sum + value, 0);
        const summary = metrics.length + ' metrics' +
            (values.length > 0
                ? ' · ' + formatQuantity(total, metrics[0]?.unit ?? 'units')
                : '');

        return this._collapsiblePanel({
            key: provider.provider + ':metrics',
            title: 'Tool metrics',
            summary,
            iconName: 'view-list-symbolic',
            bodyFactory: () => {
                const list = new St.BoxLayout({
                    vertical: true,
                    style_class: 'remaining-detail-list',
                    x_expand: true,
                });
                for (const metric of metrics) {
                    list.add_child(this._detailRow({
                        label: metric.label ?? 'Metric',
                        value: metricValueText(metric),
                        meta: [metric.period, metric.source]
                            .filter(Boolean)
                            .join(' · '),
                    }));
                }
                return list;
            },
        });
    }

    _collapsiblePanel({key, title, summary, iconName, bodyFactory}) {
        const expanded = this._expandedSections.has(key);
        const panel = new St.BoxLayout({
            vertical: true,
            style_class: expanded
                ? 'remaining-collapsible-panel remaining-collapsible-panel-expanded'
                : 'remaining-collapsible-panel',
            x_expand: true,
        });
        const button = new St.Button({
            style_class: 'remaining-collapsible-button',
            can_focus: true,
            track_hover: true,
            x_expand: true,
        });
        const content = new St.BoxLayout({
            style_class: 'remaining-collapsible-header',
            x_expand: true,
        });
        const iconBox = new St.BoxLayout({
            style_class: 'remaining-section-icon-box',
        });
        iconBox.add_child(new St.Icon({
            icon_name: iconName,
            style_class: 'remaining-section-icon',
        }));
        content.add_child(iconBox);

        const copy = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-collapsible-copy',
            x_expand: true,
        });
        copy.add_child(textLabel(
            title,
            'remaining-collapsible-title',
            {x_expand: true}
        ));
        copy.add_child(textLabel(
            summary,
            'remaining-collapsible-summary',
            {x_expand: true}
        ));
        content.add_child(copy);
        content.add_child(new St.Icon({
            icon_name: expanded ? 'go-down-symbolic' : 'go-next-symbolic',
            style_class: 'remaining-chevron-icon',
        }));
        button.set_child(content);
        button.connect('clicked', () => this._toggleSection(key));
        panel.add_child(button);

        if (expanded) {
            const body = bodyFactory();
            body.add_style_class_name('remaining-collapsible-body');
            panel.add_child(body);
        }
        return panel;
    }

    _detailRow(row) {
        const item = new St.BoxLayout({
            style_class: 'remaining-detail-row',
            x_expand: true,
        });
        const text = new St.BoxLayout({
            vertical: true,
            style_class: 'remaining-detail-text',
            x_expand: true,
        });
        text.add_child(textLabel(
            row.label,
            'remaining-detail-label',
            {x_expand: true}
        ));
        if (row.meta) {
            text.add_child(textLabel(
                row.meta,
                'remaining-detail-meta',
                {x_expand: true}
            ));
        }
        item.add_child(text);
        item.add_child(textLabel(row.value, 'remaining-detail-value'));
        return item;
    }
});

export default class RemainingBarExtension extends Extension {
    enable() {
        console.log(
            'Remaining Bar: enabled UI v' + this.metadata.version);
        this._settings = this.getSettings();
        this._indicator = new RemainingIndicator(this._settings);
        Main.panel.addToStatusArea(this.uuid, this._indicator, 1, 'right');
    }

    disable() {
        this._indicator?.destroy();
        this._indicator = null;
        this._settings = null;
    }
}
