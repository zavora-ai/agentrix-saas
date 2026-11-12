/// Typed clients for the Agentrix apps (13 §9): the owner app's API (`/api/m/v1` on the
/// platform host, bearer token from pairing) and a site's own Ask and customer account API (on
/// the site's host, W6's session).
library;

import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

/// A refusal or failure, with words for the person.
class ApiError implements Exception {
  ApiError(this.status, this.message);

  /// The HTTP status (0 when the server couldn't be reached).
  final int status;

  /// What to tell the person.
  final String message;

  /// The phone isn't paired any more.
  bool get unpaired => status == 401;

  @override
  String toString() => message;
}

/// JSON as the API sends it.
typedef Json = Map<String, dynamic>;

const _offline = "We couldn't reach your site. Check the connection and try again.";

Future<Json> _decode(http.Response r) async {
  Json body;
  try {
    final v = jsonDecode(utf8.decode(r.bodyBytes));
    body = v is Json ? v : {'value': v};
  } catch (_) {
    body = {};
  }
  if (r.statusCode >= 200 && r.statusCode < 300) return body;
  throw ApiError(r.statusCode, (body['message'] as String?) ?? "Your site couldn't finish that just now. Nothing changed. Try again in a minute.");
}

/// The owner app's API.
class OwnerApi {
  OwnerApi({required this.origin, this.token, http.Client? client}) : _http = client ?? http.Client();

  /// The platform's origin, e.g. `https://agentrix.website`.
  final String origin;

  /// The bearer token from pairing.
  String? token;

  final http.Client _http;

  Uri _u(String path) => Uri.parse('${origin.replaceAll(RegExp(r'/$'), '')}/api/m/v1$path');

  Map<String, String> get _headers => {
        'content-type': 'application/json',
        'accept': 'application/json',
        if (token != null) 'authorization': 'Bearer $token',
      };

  Future<Json> _get(String path) async {
    try {
      return await _decode(await _http.get(_u(path), headers: _headers).timeout(const Duration(seconds: 20)));
    } on ApiError {
      rethrow;
    } catch (_) {
      throw ApiError(0, _offline);
    }
  }

  Future<Json> _send(String method, String path, [Json? body]) async {
    try {
      final req = http.Request(method, _u(path))
        ..headers.addAll(_headers)
        ..body = jsonEncode(body ?? const {});
      final res = await http.Response.fromStream(await _http.send(req).timeout(const Duration(seconds: 30)));
      return await _decode(res);
    } on ApiError {
      rethrow;
    } catch (_) {
      throw ApiError(0, _offline);
    }
  }

  /// Pairs this phone with the code from Settings → Channels. Sets [token].
  Future<Json> pair(String code, String device) async {
    final j = await _send('POST', '/pair', {'code': code, 'device': device});
    token = j['token'] as String?;
    return j;
  }

  /// Who and which site.
  Future<Json> me() => _get('/me');

  /// The Now tab: greeting, brief, what needs you, Freeze.
  Future<Json> now() => _get('/now');

  /// Decisions waiting.
  Future<List<Json>> cards() async => ((await _get('/cards'))['cards'] as List).cast<Json>();

  /// One decision with its exact effect.
  Future<Json> card(String id) => _get('/cards/${Uri.encodeComponent(id)}');

  /// Approves with six digits from the authenticator app, bound to the card's fingerprint.
  Future<Json> approve(String id, String code) => _send('POST', '/cards/${Uri.encodeComponent(id)}/approve', {'code': code});

  /// Declines; nothing changes.
  Future<Json> decline(String id) => _send('POST', '/cards/${Uri.encodeComponent(id)}/decline');

  /// Freezes every WebAgent.
  Future<Json> freeze() => _send('POST', '/freeze');

  /// Something said or typed to the Chief WebAgent.
  Future<Json> ask(String said) => _send('POST', '/ask', {'said': said});

  /// Who is working on the site now (W1's presence).
  Future<Json> live() => _get('/live');

  /// The site's WebAgents.
  Future<List<Json>> webagents() async => ((await _get('/webagents'))['webagents'] as List).cast<Json>();

  /// Unpairs this phone.
  Future<void> unpair() async {
    await _send('DELETE', '/device');
    token = null;
  }
}

/// One event from a site's Ask.
class AskEvent {
  AskEvent(this.kind, this.data);

  /// `progress`, `answer` or `done`.
  final String kind;

  /// Its JSON.
  final Json data;
}

/// A site's own API on its host: Ask (the same as the page) and W6's customer sign-in.
class SiteApi {
  SiteApi({required this.origin, http.Client? client, this.session}) : _http = client ?? http.Client();

  /// The site's origin, e.g. `https://kilele-bakery-sample.agentrix.website`.
  final String origin;

  /// W6's session cookie (`name=value`), on native platforms where cookies aren't kept for us.
  String? session;

  final http.Client _http;

  Uri _u(String path) => Uri.parse('${origin.replaceAll(RegExp(r'/$'), '')}$path');

  // W6 refuses a POST that doesn't come from the site's own origin: the app is the site's own
  // first-party client, so it says so.
  Map<String, String> get _headers => {
        'content-type': 'application/json',
        'accept': 'application/json',
        'origin': origin.replaceAll(RegExp(r'/$'), ''),
        if (session != null) 'cookie': session!,
      };

  /// Asks the site. Events arrive as Ask's `progress`, then one `answer`, then `done`.
  Future<List<AskEvent>> ask(String q, {String? desk}) async {
    http.Response r;
    try {
      r = await _http
          .post(_u('/ask'), headers: {..._headers, 'accept': 'text/event-stream'}, body: jsonEncode({'q': q, 'desk': ?desk}))
          .timeout(const Duration(seconds: 40));
    } catch (_) {
      throw ApiError(0, _offline);
    }
    if (r.statusCode != 200) {
      throw ApiError(r.statusCode, 'Answers are paused right now. The site still works.');
    }
    return parseSse(utf8.decode(r.bodyBytes));
  }

  /// Sends a sign-in (or sign-up) code to `email`. Returns W6's pending step.
  Future<Json> startCode(String email, {String purpose = 'sign-in', String? name}) async {
    try {
      final r = await _http.post(_u('/account/api/code/start'), headers: _headers, body: jsonEncode({'email': email, 'purpose': purpose, 'name': ?name}));
      return _decode(r);
    } on ApiError {
      rethrow;
    } catch (_) {
      throw ApiError(0, _offline);
    }
  }

  /// Checks the code; on success keeps W6's session.
  Future<Json> verifyCode(String pending, String code) async {
    try {
      final r = await _http.post(_u('/account/api/code/verify'), headers: _headers, body: jsonEncode({'pending': pending, 'code': code}));
      final cookie = r.headers['set-cookie'];
      if (cookie != null) session = cookie.split(';').first;
      return _decode(r);
    } on ApiError {
      rethrow;
    } catch (_) {
      throw ApiError(0, _offline);
    }
  }

  /// Whether someone is signed in, and who.
  Future<Json> currentSession() async {
    try {
      return _decode(await _http.get(_u('/account/api/session'), headers: _headers));
    } catch (_) {
      return {'signed_in': false};
    }
  }
}

/// Reads a `text/event-stream` body.
List<AskEvent> parseSse(String body) {
  final out = <AskEvent>[];
  var event = '';
  final data = StringBuffer();
  void flush() {
    if (data.isNotEmpty) {
      try {
        final v = jsonDecode(data.toString());
        out.add(AskEvent(event, v is Json ? v : {'value': v}));
      } catch (_) {}
    }
    event = '';
    data.clear();
  }

  for (final line in const LineSplitter().convert(body)) {
    if (line.isEmpty) {
      flush();
    } else if (line.startsWith('event:')) {
      event = line.substring(6).trim();
    } else if (line.startsWith('data:')) {
      if (data.isNotEmpty) data.write('\n');
      data.write(line.substring(5).trimLeft());
    }
  }
  flush();
  return out;
}
