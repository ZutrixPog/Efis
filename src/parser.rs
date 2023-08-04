use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{char, digit1, none_of},
    combinator::{map_res, recognize, opt},
    multi::{separated_list1, many0, many1},
    sequence::preceded,
    IResult,
};

#[derive(Debug, PartialEq)]
pub enum EfisCommand<'a> {
    Set(&'a str, &'a str, Option<u64>),
    Get(&'a str),
    Del(&'a str),
    Incr(&'a str),
    Decr(&'a str),
    Expire(&'a str, u64),
    TTL(&'a str),
    LPush(&'a str, Vec<&'a str>),
    RPush(&'a str, Vec<&'a str>),
    LPop(&'a str),
    RPop(&'a str),
    SAdd(&'a str, Vec<&'a str>),
    SMembers(&'a str),
    ZAdd(&'a str, &'a str, &'a str),
    ZRange(&'a str, u64, u64),
    Publish(&'a str, &'a str),
    Subscribe(&'a str),
    Unknown(&'a str),
}

pub fn parse_command(input: &str) -> IResult<&str, EfisCommand> {
    alt((
        parse_set_command,
        parse_get_command,
        parse_del_command,
        parse_incr_command,
        parse_decr_command,
        parse_expire_command,
        parse_ttl_command,
        parse_lpush_command,
        parse_rpush_command,
        parse_lpop_command,
        parse_rpop_command,
        parse_sadd_command,
        parse_smembers_command,
        parse_zadd_command,
        parse_zrange_command,
        parse_publish_command,
        parse_subscribe_command,
        parse_unknown_command,
    ))(input)
}


fn parse_set_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("SET")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, value) = parse_token(input)?;
    let (input, ttl) = parse_ex_option(input)?;
    Ok((input, EfisCommand::Set(key, value, ttl)))
}

fn parse_ex_option(input: &str) -> IResult<&str, Option<u64>> {
    let (input, _) = opt(preceded(parse_whitespace, tag("EX")))(input)?;
    let (input, ttl) = opt(parse_whitespace_then_number)(input)?;
    Ok((input, ttl))
}

fn parse_get_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("GET")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    Ok((input, EfisCommand::Get(key)))
}

fn parse_del_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("DEL")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    Ok((input, EfisCommand::Del(key)))
}

fn parse_incr_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("INCR")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    Ok((input, EfisCommand::Incr(key)))
}

fn parse_decr_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("DECR")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    Ok((input, EfisCommand::Decr(key)))
}

fn parse_expire_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("EXPIRE")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, duration) = parse_number(input)?;
    Ok((input, EfisCommand::Expire(key, duration)))
}

fn parse_ttl_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("TTL")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    Ok((input, EfisCommand::TTL(key)))
}

fn parse_lpush_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("LPUSH")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, elements) = parse_token_list(input)?;
    Ok((input, EfisCommand::LPush(key, elements)))
}

fn parse_rpush_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("RPUSH")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, elements) = parse_token_list(input)?;
    Ok((input, EfisCommand::RPush(key, elements)))
}

fn parse_lpop_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("LPOP")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    Ok((input, EfisCommand::LPop(key)))
}

fn parse_rpop_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("RPOP")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    Ok((input, EfisCommand::RPop(key)))
}

fn parse_sadd_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("SADD")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, members) = parse_token_list(input)?;
    Ok((input, EfisCommand::SAdd(key, members)))
}

fn parse_smembers_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("SMEMBERS")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    Ok((input, EfisCommand::SMembers(key)))
}

fn parse_zadd_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("ZADD")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, score) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, member) = parse_token(input)?;
    Ok((input, EfisCommand::ZAdd(key, score, member)))
}

fn parse_zrange_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("ZRANGE")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, key) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, start) = parse_number(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, stop) = parse_number(input)?;
    Ok((input, EfisCommand::ZRange(key, start, stop)))
}

fn parse_publish_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("PUBLISH")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, channel) = parse_token(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, message) = parse_token(input)?;
    Ok((input, EfisCommand::Publish(channel, message)))
}

fn parse_subscribe_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, _) = tag("SUBSCRIBE")(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, channel) = parse_token(input)?;
    Ok((input, EfisCommand::Subscribe(channel)))
}

fn parse_unknown_command(input: &str) -> IResult<&str, EfisCommand> {
    let (input, cmd) = parse_token(input)?;
    Ok((input, EfisCommand::Unknown(cmd)))
}

fn parse_whitespace(input: &str) -> IResult<&str, &str> {
    recognize(many0(alt((char(' '), char('\t'), char('\r'), char('\n')))))(input)
}

fn parse_token(input: &str) -> IResult<&str, &str> {
    recognize(many1(none_of(" \t\r\n")))(input)
}

fn parse_token_list(input: &str) -> IResult<&str, Vec<&str>> {
    separated_list1(char(' '), parse_token)(input)
}

fn parse_number(input: &str) -> IResult<&str, u64> {
    map_res(recognize(digit1), |s: &str| s.parse::<u64>())(input)
}

fn parse_whitespace_then_number(input: &str) -> IResult<&str, u64> {
    preceded(parse_whitespace, parse_number)(input)
}

fn parse_ttl_option(input: &str) -> IResult<&str, Option<u64>> {
    let (input, _) = preceded(parse_whitespace, tag("TTL"))(input)?;
    let (input, _) = parse_whitespace(input)?;
    let (input, ttl) = parse_number(input)?;
    Ok((input, Some(ttl)))
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn test_parse_set_command_with_expiry() {
        let input = "SET mykey myvalue EX 60";
        let expected = Ok(("", EfisCommand::Set("mykey", "myvalue", Some(60))));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_set_command_without_expiry() {
        let input = "SET mykey myvalue";
        let expected = Ok(("", EfisCommand::Set("mykey", "myvalue", None)));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_get_command() {
        let input = "GET mykey";
        let expected = Ok(("", EfisCommand::Get("mykey")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_del_command() {
        let input = "DEL mykey";
        let expected = Ok(("", EfisCommand::Del("mykey")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_incr_command() {
        let input = "INCR mykey";
        let expected = Ok(("", EfisCommand::Incr("mykey")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_decr_command() {
        let input = "DECR mykey";
        let expected = Ok(("", EfisCommand::Decr("mykey")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_expire_command() {
        let input = "EXPIRE mykey 60";
        let expected = Ok(("", EfisCommand::Expire("mykey", 60)));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_ttl_command() {
        let input = "TTL mykey";
        let expected = Ok(("", EfisCommand::TTL("mykey")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_lpush_command() {
        let input = "LPUSH mylist value1 value2 value3";
        let expected = Ok((
            "",
            EfisCommand::LPush("mylist", vec!["value1", "value2", "value3"]),
        ));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_rpush_command() {
        let input = "RPUSH mylist value1 value2 value3";
        let expected = Ok((
            "",
            EfisCommand::RPush("mylist", vec!["value1", "value2", "value3"]),
        ));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_lpop_command() {
        let input = "LPOP mylist";
        let expected = Ok(("", EfisCommand::LPop("mylist")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_rpop_command() {
        let input = "RPOP mylist";
        let expected = Ok(("", EfisCommand::RPop("mylist")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_sadd_command() {
        let input = "SADD myset value1 value2 value3";
        let expected = Ok((
            "",
            EfisCommand::SAdd("myset", vec!["value1", "value2", "value3"]),
        ));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_smembers_command() {
        let input = "SMEMBERS myset";
        let expected = Ok(("", EfisCommand::SMembers("myset")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_zadd_command() {
        let input = "ZADD 3 2 member1";
        let expected = Ok(("", EfisCommand::ZAdd("3", "2", "member1")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_zrange_command() {
        let input = "ZRANGE myzset 0 10";
        let expected = Ok(("", EfisCommand::ZRange("myzset", 0, 10)));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_publish_command() {
        let input = "PUBLISH mychannel mymessage";
        let expected = Ok(("", EfisCommand::Publish("mychannel", "mymessage")));
        assert_eq!(parse_command(input), expected);
    }

    #[test]
    fn test_parse_unknown_command() {
        let input = "UNKNOWN";
        let expected = Ok(("", EfisCommand::Unknown("UNKNOWN")));
        assert_eq!(parse_command(input), expected);
    }
}