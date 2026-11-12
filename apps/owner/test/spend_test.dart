import 'package:agentrix_owner/src/now.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('the brief says this month\'s AI spend in the site\'s currency', () {
    expect(
      spendLine({'used': 38, 'budget': 3870, 'currency': 'KES', 'period': 'month', 'words': 'KSh 38 of KSh 3,870 this month'}),
      'AI spend KSh 38 of KSh 3,870 this month.',
    );
    expect(spendLine({'used': 38, 'budget': null, 'currency': 'KES', 'period': 'month', 'words': 'KSh 38 this month'}), 'AI spend KSh 38 this month.');
  });

  test('spend the server couldn\'t read is left out, never guessed', () {
    expect(spendLine({'used': null, 'budget': null, 'currency': null, 'period': 'month', 'words': null}), isNull);
    expect(spendLine({'used': 0.21, 'budget': 5, 'currency': 'USD'}), isNull, reason: 'an older server sent today\'s dollars');
    expect(spendLine(null), isNull);
  });
}
