use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

/// プロパティの値を表す列挙型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropertyValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    /// 日付 (1970-01-01 からの日数)
    Date(i32),
    /// 日時 (Unix エポックからのミリ秒)
    DateTime(i64),
    /// 期間 (ISO 8601)
    Duration {
        months: i32,
        days: i32,
        millis: i64,
    },
    /// リスト（`tags: ['a', 'b']`）。要素は null・リスト以外のプリミティブ
    /// （Neo4j と同じくネストしたリストやマップは保存できない）。
    List(Vec<PropertyValue>),
}

impl PartialEq for PropertyValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PropertyValue::Null, PropertyValue::Null) => true,
            (PropertyValue::Bool(a), PropertyValue::Bool(b)) => a == b,
            (PropertyValue::Int(a), PropertyValue::Int(b)) => a == b,
            (PropertyValue::Float(a), PropertyValue::Float(b)) => a.to_bits() == b.to_bits(),
            (PropertyValue::String(a), PropertyValue::String(b)) => a == b,
            (PropertyValue::Date(a), PropertyValue::Date(b)) => a == b,
            (PropertyValue::DateTime(a), PropertyValue::DateTime(b)) => a == b,
            (
                PropertyValue::Duration {
                    months: ma,
                    days: da,
                    millis: msa,
                },
                PropertyValue::Duration {
                    months: mb,
                    days: db,
                    millis: msb,
                },
            ) => ma == mb && da == db && msa == msb,
            (PropertyValue::List(a), PropertyValue::List(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for PropertyValue {}

impl Hash for PropertyValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            PropertyValue::Null => {}
            PropertyValue::Bool(v) => v.hash(state),
            PropertyValue::Int(v) => v.hash(state),
            PropertyValue::Float(v) => v.to_bits().hash(state),
            PropertyValue::String(v) => v.hash(state),
            PropertyValue::Date(v) => v.hash(state),
            PropertyValue::DateTime(v) => v.hash(state),
            PropertyValue::Duration {
                months,
                days,
                millis,
            } => {
                months.hash(state);
                days.hash(state);
                millis.hash(state);
            }
            PropertyValue::List(items) => items.hash(state),
        }
    }
}

impl From<bool> for PropertyValue {
    fn from(v: bool) -> Self {
        PropertyValue::Bool(v)
    }
}

impl From<i64> for PropertyValue {
    fn from(v: i64) -> Self {
        PropertyValue::Int(v)
    }
}

impl From<i32> for PropertyValue {
    fn from(v: i32) -> Self {
        PropertyValue::Int(v as i64)
    }
}

impl From<f64> for PropertyValue {
    fn from(v: f64) -> Self {
        PropertyValue::Float(v)
    }
}

impl From<String> for PropertyValue {
    fn from(v: String) -> Self {
        PropertyValue::String(v)
    }
}

impl From<&str> for PropertyValue {
    fn from(v: &str) -> Self {
        PropertyValue::String(v.to_string())
    }
}

impl std::fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropertyValue::Null => write!(f, "null"),
            PropertyValue::Bool(v) => write!(f, "{}", v),
            PropertyValue::Int(v) => write!(f, "{}", v),
            PropertyValue::Float(v) => write!(f, "{}", v),
            PropertyValue::String(v) => write!(f, "\"{}\"", v),
            PropertyValue::Date(days) => {
                let (y, m, d) = temporal::days_to_ymd(*days);
                write!(f, "{:04}-{:02}-{:02}", y, m, d)
            }
            PropertyValue::DateTime(ms) => {
                let (y, mo, d, h, mi, s, frac) = temporal::millis_to_datetime(*ms);
                write!(
                    f,
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                    y, mo, d, h, mi, s, frac
                )
            }
            PropertyValue::Duration {
                months,
                days,
                millis,
            } => {
                write!(
                    f,
                    "{}",
                    temporal::duration_to_string(*months, *days, *millis)
                )
            }
            PropertyValue::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
        }
    }
}

/// テンポラル型の補助関数
pub mod temporal {
    /// 1970-01-01 からの日数を (year, month, day) に変換
    pub fn days_to_ymd(days: i32) -> (i32, i32, i32) {
        let jdn = days as i64 + 2_440_588;
        let a = jdn + 32_044;
        let b = (4 * a + 3) / 146_097;
        let c = a - (146_097 * b) / 4;
        let d = (4 * c + 3) / 1461;
        let e = c - (1461 * d) / 4;
        let m = (5 * e + 2) / 153;
        let day = (e - (153 * m + 2) / 5 + 1) as i32;
        let month = (m + 3 - 12 * (m / 10)) as i32;
        let year = (100 * b + d - 4800 + m / 10) as i32;
        (year, month, day)
    }

    /// (year, month, day) を 1970-01-01 からの日数に変換
    pub fn ymd_to_days(year: i32, month: i32, day: i32) -> i32 {
        let y = year as i64;
        let m = month as i64;
        let d = day as i64;
        let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
        let a = y / 100;
        let b = 2 - a + a / 4;
        let jdn =
            (365.25 * (y + 4716) as f64) as i64 + (30.6001 * (m + 1) as f64) as i64 + d + b - 1524;
        (jdn - 2_440_588) as i32
    }

    /// Unix ミリ秒を (year, month, day, hour, min, sec, millis_frac) に変換
    pub fn millis_to_datetime(ms: i64) -> (i32, i32, i32, i32, i32, i32, i32) {
        let total_secs = ms.div_euclid(1000);
        let frac = ms.rem_euclid(1000) as i32;
        let days = total_secs.div_euclid(86400) as i32;
        let secs_in_day = total_secs.rem_euclid(86400) as i32;
        let (y, mo, d) = days_to_ymd(days);
        let h = secs_in_day / 3600;
        let mi = (secs_in_day % 3600) / 60;
        let s = secs_in_day % 60;
        (y, mo, d, h, mi, s, frac)
    }

    /// (year, month, day, hour, min, sec, millis_frac) を Unix ミリ秒に変換
    pub fn datetime_to_millis(y: i32, mo: i32, d: i32, h: i32, mi: i32, s: i32, frac: i32) -> i64 {
        let days = ymd_to_days(y, mo, d) as i64;
        days * 86_400_000
            + h as i64 * 3_600_000
            + mi as i64 * 60_000
            + s as i64 * 1_000
            + frac as i64
    }

    /// 扱える年の範囲（日数を `i32` で持つので、その範囲に収まる幅にしている）。
    pub const YEAR_RANGE: std::ops::RangeInclusive<i32> = -999_999..=999_999;

    /// 実在する日付か（月は 1–12、日はその月の末日まで。うるう年を考える）。
    pub fn is_valid_date(year: i32, month: i32, day: i32) -> bool {
        YEAR_RANGE.contains(&year)
            && (1..=12).contains(&month)
            && (1..=days_in_month(year, month)).contains(&day)
    }

    /// 実在する日付なら 1970-01-01 からの日数を返す。
    pub fn checked_ymd_to_days(year: i32, month: i32, day: i32) -> Option<i32> {
        is_valid_date(year, month, day).then(|| ymd_to_days(year, month, day))
    }

    /// "YYYY-MM-DD" 文字列を日数に変換（存在しない日付は `None`）
    pub fn parse_date(s: &str) -> Option<i32> {
        // 先頭の '-' は紀元前の年の符号
        let (sign, rest) = match s.strip_prefix('-') {
            Some(r) => (-1, r),
            None => (1, s),
        };
        let parts: Vec<&str> = rest.split('-').collect();
        if parts.len() != 3 {
            return None;
        }
        let y: i32 = parts[0].parse::<i32>().ok()? * sign;
        let m: i32 = parts[1].parse().ok()?;
        let d: i32 = parts[2].parse().ok()?;
        checked_ymd_to_days(y, m, d)
    }

    /// "YYYY-MM-DDTHH:MM:SS[.fff][Z]" 文字列を Unix ミリ秒に変換
    pub fn parse_datetime(s: &str) -> Option<i64> {
        let s = s.trim_end_matches('Z');
        let pos = s.find('T')?;
        let (date_part, time_part) = (&s[..pos], &s[pos + 1..]);
        let days = parse_date(date_part)?;
        let time_parts: Vec<&str> = time_part.split(':').collect();
        if time_parts.len() < 2 {
            return None;
        }
        let h: i32 = time_parts[0].parse().ok()?;
        let m: i32 = time_parts[1].parse().ok()?;
        if !(0..24).contains(&h) || !(0..60).contains(&m) {
            return None;
        }
        let (s, frac) = if time_parts.len() >= 3 {
            let sec_part = time_parts[2];
            if let Some(dot) = sec_part.find('.') {
                let s: i32 = sec_part[..dot].parse().ok()?;
                let frac_str = &sec_part[dot + 1..];
                if frac_str.is_empty() || !frac_str.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                // ミリ秒までを使う（それより細かい桁は切り捨て）
                let ms_digits = format!("{:0<3}", &frac_str[..frac_str.len().min(3)]);
                (s, ms_digits.parse().ok()?)
            } else {
                let s: i32 = sec_part.parse().ok()?;
                (s, 0)
            }
        } else {
            (0, 0)
        };
        if !(0..60).contains(&s) {
            return None;
        }
        Some(datetime_to_millis(
            days_to_ymd(days).0,
            days_to_ymd(days).1,
            days_to_ymd(days).2,
            h,
            m,
            s,
            frac,
        ))
    }

    /// "P[nY][nM][nD][T[nH][nM][nS]]" を (months, days, millis) に変換。
    /// 書式が誤っているときや、各成分が範囲（月・日は `i32`、ミリ秒は `i64`）を超えるときは `None`。
    pub fn parse_duration(s: &str) -> Option<(i32, i32, i64)> {
        if !s.starts_with('P') {
            return None;
        }
        let s = &s[1..];
        let mut months = 0i32;
        let mut days = 0i32;
        let mut millis = 0i64;

        let (date_part, time_part) = if let Some(t_pos) = s.find('T') {
            (&s[..t_pos], Some(&s[t_pos + 1..]))
        } else {
            (s, None)
        };

        let mut num_buf = String::new();
        for ch in date_part.chars() {
            if ch.is_ascii_digit() || ch == '.' {
                num_buf.push(ch);
            } else {
                let n: f64 = if num_buf.is_empty() {
                    0.0
                } else {
                    num_buf.parse().ok()?
                };
                num_buf.clear();
                let (target, unit) = match ch {
                    'Y' => (&mut months, 12.0),
                    'M' => (&mut months, 1.0),
                    'W' => (&mut days, 7.0),
                    'D' => (&mut days, 1.0),
                    _ => return None,
                };
                *target = target.checked_add(f64_to_i32(n * unit)?)?;
            }
        }

        if let Some(tp) = time_part {
            let mut num_buf = String::new();
            for ch in tp.chars() {
                if ch.is_ascii_digit() || ch == '.' {
                    num_buf.push(ch);
                } else {
                    let n: f64 = if num_buf.is_empty() {
                        0.0
                    } else {
                        num_buf.parse().ok()?
                    };
                    num_buf.clear();
                    let unit = match ch {
                        'H' => 3_600_000.0,
                        'M' => 60_000.0,
                        'S' => 1_000.0,
                        _ => return None,
                    };
                    millis = millis.checked_add(f64_to_i64(n * unit)?)?;
                }
            }
        }

        Some((months, days, millis))
    }

    /// Duration を ISO 8601 文字列に変換
    pub fn duration_to_string(months: i32, days: i32, millis: i64) -> String {
        let years = months / 12;
        let rem_months = months % 12;
        let hours = millis / 3_600_000;
        let rem = millis % 3_600_000;
        let minutes = rem / 60_000;
        let secs = (rem % 60_000) / 1_000;
        let frac = rem % 1_000;

        let mut s = String::from("P");
        if years != 0 {
            s.push_str(&format!("{}Y", years));
        }
        if rem_months != 0 {
            s.push_str(&format!("{}M", rem_months));
        }
        if days != 0 {
            s.push_str(&format!("{}D", days));
        }
        if hours != 0 || minutes != 0 || secs != 0 || frac != 0 {
            s.push('T');
            if hours != 0 {
                s.push_str(&format!("{}H", hours));
            }
            if minutes != 0 {
                s.push_str(&format!("{}M", minutes));
            }
            if frac != 0 {
                s.push_str(&format!("{}.{:03}S", secs, frac));
            } else if secs != 0 {
                s.push_str(&format!("{}S", secs));
            }
        }
        if s == "P" {
            s.push_str("T0S");
        }
        s
    }

    fn f64_to_i32(n: f64) -> Option<i32> {
        (i32::MIN as f64..=i32::MAX as f64)
            .contains(&n)
            .then_some(n as i32)
    }

    fn f64_to_i64(n: f64) -> Option<i64> {
        // i64::MAX は f64 で正確に表せないので、2^63 未満で判定する
        (-9.223_372_036_854_776e18..9.223_372_036_854_776e18)
            .contains(&n)
            .then_some(n as i64)
    }

    /// Date に Duration を加算する。結果が扱える範囲（[`YEAR_RANGE`]）を超えるときは `None`。
    /// 月を足して月末を超える日は、その月の末日に丸める（1/31 + 1 か月 = 2/28 または 2/29）。
    pub fn add_duration_to_date(days: i32, months: i32, dur_days: i32, millis: i64) -> Option<i32> {
        let (y, m, d) = days_to_ymd(days);
        let total_months = (y as i64 * 12 + m as i64 - 1) + months as i64;
        let new_year = i32::try_from(total_months.div_euclid(12)).ok()?;
        let new_month = total_months.rem_euclid(12) as i32 + 1;
        if !YEAR_RANGE.contains(&new_year) {
            return None;
        }
        let max_day = days_in_month(new_year, new_month);
        let new_days = ymd_to_days(new_year, new_month, d.min(max_day)) as i64;
        let result = new_days + dur_days as i64 + millis / 86_400_000;
        let result = i32::try_from(result).ok()?;
        YEAR_RANGE
            .contains(&days_to_ymd(result).0)
            .then_some(result)
    }

    /// DateTime（Unix ミリ秒）に Duration を加算する。範囲を超えるときは `None`。
    pub fn add_duration_to_datetime(ms: i64, months: i32, days: i32, millis: i64) -> Option<i64> {
        let (y, mo, d, h, mi, s, frac) = millis_to_datetime(ms);
        let new_days = add_duration_to_date(ymd_to_days(y, mo, d), months, days, 0)?;
        let (ny, nmo, nd) = days_to_ymd(new_days);
        datetime_to_millis(ny, nmo, nd, h, mi, s, frac).checked_add(millis)
    }

    fn days_in_month(year: i32, month: i32) -> i32 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                    29
                } else {
                    28
                }
            }
            _ => 30,
        }
    }
}
