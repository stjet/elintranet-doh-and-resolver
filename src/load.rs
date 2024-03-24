use std::io::{ prelude::*, BufReader };
use std::fs::File;
use std::collections::HashMap;

use crate::utils;

pub fn load_env() -> HashMap<String, String> {
  let mut buf_reader = BufReader::new(File::open(".env").unwrap());
  let mut contents = String::new();
  buf_reader.read_to_string(&mut contents).unwrap();

  let mut env: HashMap<String, String> = HashMap::new();

  let lines: Vec<&str> = contents.split('\n').collect();

  for line in lines {
    let parts: Vec<&str> = line.split('=').collect();
    if parts.len() == 1 {
      continue;
    }
    env.insert(parts[0].to_string(), parts[1..].join("="));
  }

  return env;
}

#[derive(Clone)]
pub struct SubdomainInfo {
  pub ip: [u8; 4],
  pub ip_string: String,
  pub port: Option<u16>,
}

pub fn get_intranet_subdomains() -> HashMap<String, SubdomainInfo> {
  let mut buf_reader = BufReader::new(File::open("intranet_subdomains.csv").unwrap());
  let mut contents = String::new();
  buf_reader.read_to_string(&mut contents).unwrap();

  let mut intranet_subdomains: HashMap<String, SubdomainInfo> = HashMap::new();

  let lines: Vec<&str> = contents.split('\n').collect();

  for line in lines {
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() == 1 {
      continue;
    }
    let ip_parts: Vec<&str> = parts[1].split(":").collect(); //split ip and port, if any port
    let port = {
      if ip_parts.len() == 1 {
        None
      } else {
        Some(ip_parts[1].parse::<u16>().unwrap())
      }
    };
    let ip_string = ip_parts[0].to_string();
    intranet_subdomains.insert(parts[0].to_string(), SubdomainInfo { ip: utils::ip_string_to_u8_array(&ip_string), ip_string, port });
  }

  intranet_subdomains
}
