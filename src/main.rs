use std::collections::HashMap;
use std::str::FromStr;

use tiny_http::{ Server, SslConfig, Response, Header, Method };
use reqwest::header::{ HeaderMap, ACCEPT, CONTENT_TYPE };

mod load;
mod url;
mod utils;

//ok so for whatever FUCKING reason (maybe security?), I think firefox blocks A records that point in the 192.168.x.x range (only 80% sure)...
//but ONLY for DoH??? not regular DNS. absolutely bizzare
//so I wasted several hours debugging and trying to figure out what was wrong with my dns message response
//I am pissed y'all.
//So I think instead, if the ip address in intranet_subdomains.csv starts with 192.168.x.x, we will return our own ip, and proxy it. advantage is then we can specify a port too

//how to run: cargo build --release && sudo ./target/release/elintranet-doh-and-resolver

//check for endianness problems? shouldn't be any...?

//rfc 1035 (section 4, section 7.3)
//rfc 8484
//www.tcpipguide.com/free/t_DNSMessageHeaderandQuestionSectionFormat-2.htm

fn get_header(find_header: String, headers: &[Header]) -> Option<String> {
  for header in headers {
    if header.field.as_str() == find_header {
      return Some(header.value.to_string());
    }
  }
  return None;
}

fn extract_subdomain<'a>(host: &'a str, intranet_host: &'a str) -> &'a str {
  //println!("{} {}", host, intranet_host);
  host.get(..host.len() - intranet_host.len() - 1).unwrap()
}

fn is_intranet_subdomain(subdomain: &str, intranet_subdomains: &HashMap<String, load::SubdomainInfo>) -> bool {
  intranet_subdomains.keys().collect::<Vec<&String>>().contains(&&subdomain.to_string())
}

enum QueryError {
  NXDomain,
  NonIntranet,
}

//only supports returning A record currently
fn do_dns_query(host: &str, internal_ip: String, intranet_host: &String, intranet_subdomains: &HashMap<String, load::SubdomainInfo>) -> Result<[u8; 4], QueryError> {
  if host == "dns.".to_string() + intranet_host {
    //hey, that's us! return our ip address
    Ok(utils::ip_string_to_u8_array(&internal_ip))
  } else if host.ends_with(intranet_host) {
    //direct request to intranet site, return ip address of the subdomain
    if let Some(sub_ip) = intranet_subdomains.get(extract_subdomain(&host, &intranet_host)).cloned() {
      Ok(sub_ip.ip)
    } else {
      Err(QueryError::NXDomain)
    }
  } else if is_intranet_subdomain(&host, &intranet_subdomains) {
    //request to intranet tld (that will redirect to intranet subdomain),
    //return our ip address
    Ok(utils::ip_string_to_u8_array(&internal_ip))
  } else {
    Err(QueryError::NonIntranet)
  }
}

//start param is a different index to start at (used for pointers)
//not the biggest fan of usize but whatever man
//WORKS!!
fn extract_host_from_dns_query(dns_query: &[u8], start: Option<usize>) -> Result<String, ()> {
  //13th byte should be start of question, indicate how long the label/zone is in bytes
  let mut length_pos: usize = 12;
  if start.is_some() {
    length_pos = start.unwrap();
  }
  let mut domain_name: Vec<String> = Vec::new();
  let query_len = dns_query.len();
  loop {
    if length_pos >= query_len {
      return Err(());
    }
    let length = dns_query[length_pos];
    if length_pos + usize::from(length) >= dns_query.len() {
      return Err(());
    }
    if length == 0 {
      //the last ., ended
      break;
    }
    let label;
    if length >= 192 {
      //first two bits are 11, is pointer
      let offset: usize = utils::binary_to_u8(&utils::to_binary(length, false)[2..]).into();
      label = extract_host_from_dns_query(dns_query, Some(offset))?;
    } else {
      //ascii
      let label_bytes = &dns_query[(length_pos + 1)..=(length_pos + usize::from(length))];
      label = label_bytes.iter().map(|c_u8| char::from(c_u8.clone()).to_string()).collect::<Vec<String>>().join("");
    }
    domain_name.push(label);
    length_pos = length_pos + usize::from(length) + 1;
  }
  return Ok(domain_name.join("."));
}

//16 bits (2 bytes)
/*
fn extract_type_bytes_from_dns_query(dns_query: &[u8]) -> u16 {
  //
  let length = dns_query[12];
  if length >= 192 {
    ((dns_query[13] as u16) << 8) | (dns_query[14] as u16)
  } else {
    ((dns_query[12 + length as usize] as u16) << 8) | (dns_query[12 + length as usize + 1] as u16)
  }
}
*/

fn query_hostname_to_label_bytes(query_hostname: &str) -> Vec<u8> {
  let mut label_bytes = Vec::new();
  //length
  //error if too long?
  //
  for label in query_hostname.split(".") {
    label_bytes.push(label.len() as u8);
    for c in label.chars() {
      //convert to ascii
      label_bytes.push(c as u8);
    }
  }
  label_bytes.push(0);
  label_bytes
}

fn main() {
  //println!("{}", extract_host_from_dns_query(&vec![0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 3, 119, 119, 119, 7, 101, 120, 97, 109, 112, 108, 101, 3, 99, 111, 109, 0, 0, 1, 0, 1], None).unwrap());
  let env = load::load_env();

  println!("{:?}", env);

  let INTRANET_HOST = env.get(&"INTRANET_HOST".to_string()).unwrap();
  let INTERNAL_IP = env.get(&"INTERNAL_IP".to_string()).unwrap();
  let PORT = 443; //8443 to run without sudo? but then needs to specify port in doh config

  let NON_INTRANET_DOH = env.get(&"NON_INTRANET_DOH".to_string()).cloned().unwrap_or("https://query.hdns.io/dns-query".to_string());

  println!("Running on dns.{}:{}", INTRANET_HOST, PORT);

  let intranet_subdomains = load::get_intranet_subdomains();

  let client = reqwest::blocking::Client::new();
  let self_cert_client = reqwest::blocking::Client::builder().add_root_certificate(reqwest::Certificate::from_pem(include_bytes!("../fakeca.pem")).expect("Did not find root CA cert")).build().unwrap();

  //let server = Server::http(INTERNAL_IP.to_owned() + ":" + &PORT.to_string()).unwrap();
  let ssl_config = SslConfig {
    certificate: include_bytes!("../domain.crt").to_vec(),
    private_key: include_bytes!("../domain.key").to_vec(),
  };
  let server = Server::https(INTERNAL_IP.to_owned() + ":" + &PORT.to_string(), ssl_config).unwrap();

  for mut request in server.incoming_requests() {
    println!("url: {:?} method {:?} headers: {:?}", request.url(), request.method(), request.headers());
    let method = request.method();
    let req_url = request.url();
    let path = url::get_path(req_url);
    let maybe_host = get_header("Host".to_string(), request.headers());

    if maybe_host.is_some() {
      let host = maybe_host.unwrap();
      println!("\nDirect HTTP request: {}{}\n", host, path);
      //this part is the resolver
      let dns_host = "dns.".to_string() + INTRANET_HOST;
      if host == dns_host || host == dns_host + ":" + &PORT.to_string() {
        //let maybe_auth = get_header("Authorization".to_string(), request.headers());
        //dns request, yay
        //https://www.rfc-editor.org/rfc/rfc8484
        if path == "/dns-query" {
          //this part is the DoH stuff
          if method == &Method::Get || method == &Method::Post {
            let dns_query: Option<Vec<u8>>;
            if method == &Method::Get {
              //read query param
              let queries = url::get_queries(req_url);
              if let Some(query) = queries.get(&"dns".to_string()) {
                //base64 to bytes
                if let Ok(u8_vec) = utils::b64_url_to_u8_vec(query) {
                  dns_query = Some(u8_vec);
                } else {
                  //error parsing the b64
                  dns_query = None;
                }
              } else {
                dns_query = None;
              }
            } else if method == &Method::Post {
              if Some("application/dns-message".to_string()) == get_header("Content-Type".to_string(), request.headers()) {
                //read post data
                let mut post_data = Vec::new();
                request.as_reader().read_to_end(&mut post_data).unwrap();
                dns_query = Some(post_data);
              } else {
                dns_query = None;
              }
            } else {
              dns_query = None;
            }
            if dns_query.is_some() {
              let dns_query = dns_query.unwrap(); //u8 vec
              //Identification (not needed for DoH, should be 0), 16 bits
              /*
              Flags (total 16 bits)
              QR (query: 0, reply: 1), 1 bit
              OPCODE (standard: 0, inverse: 1, status: 2), 4 bits (opcode in query is repeated in response)
              AA (if authorative answer for hostname), 1 bit
              TC (whether message was truncated), 1 bit
              RD (where recursion desired), 1 bit
              RA (in response, whether recursion available), 1 bit
              Z (reserved), 3 bits
              RCODE (response code, NOERROR: 0, FORM(at)ERR: 1, SERVFAIL: 2, NXDOMAIN: 3), 4 bits
              */
              //# of questions, 16 bits
              //# of answers (response), 16 bits
              //# of authority resource records (aka RR), 16 bits
              //# of additional RRs, 16 bits
              //so above is 12 bytes

              //what the first 12 bytes will be for a regular response (4th [0 -> 3] and 8th [1 -> 0] byte will be different for NXDOMAIN, similar for SERVFAIL)
              //Id: 0, 0. Flags: 1 0000 0 0 0, 0 000 0000 (or 0011 for NXDOMAIN, 0010 for SERVFAIL). # of q: 0, 1 (question in query apparently needs to be copied to response). # of a: 0 1 (0 if NXDOMAIN, SERVFAIL). # of aurr: 0 0. # of adrr: 0 0.
              //ok so above is 128 0 but everyone seems to be doing 129 128
              let mut resp_start_bytes: Vec<u8> = vec![0, 0, 129, 128, 0, 0, 0, 1, 0, 0, 0, 0];
              /*
              Question section
              Can have many questions, but in practice most only allow 1. so we will do same, plus its easier (see "# of questions" field)
              IMPORTANT: the question in the query is apparently COPIED TO THE RESPONSE too, so response's QDCOUNT will be 1 and contain the question
              Format:
              NAME, variable length
              - NAME is divided into multiple zones/labels (eg: en, wikipedia, org), each zone has 8 bits indicating how many bytes the length is (eg, en is 2 bytes), then the name in ascii, repeated), 0 indicates the NAME is done. Eg: 2 en 9 wikipedia 3 org 0
              - labels start with 00
              - labels start with length of label
              - NAME or a zone can also be a pointer (16 bits) if first two bits are 11, other 14 bits are an offset
              TYPE (of question. A, AAAA, MX, TXT, or special from 251-255, 255 is * [all records]), 16 bits
              CLASS (probably IN for internet), 16 bits
              */
              /*
              Answer section
              Can have many answer, but in our case only one (see "# of answers" field)
              RR Format:
              NAME, variable length (see above in question section)
              TYPE (A, AAAA, MX, TXT, etc), 16 bits
              CLASS, 16 bits
              TTL (time record is valid for), 32 bits
              RDLENGTH (length of RDATA in bytes), 16 bits
              RDATA (additional RR specific data, see RDLENGTH), variable length
              */
              //Authority section: RRs that point toward authority (NOT RELEVANT for us)
              //Additional space section: RRs with additional information (NOT RELEVANT for us)
              //first do some sanity checks, extract ID (should be 0 but w/e), make sure is query
              //also only accept if one question (pretty sure no one does multiple nowadays anyways)
              //also check length
              //TODO
              //
              //println!("q: {:?}", dns_query);
              //extract the host name
              let query_host_wrapped = extract_host_from_dns_query(&dns_query, None);
              if let Ok(query_host) = query_host_wrapped {
                println!("\nRequested: {}\n", query_host);
                //now actual dns query stuff, and http response
                match do_dns_query(&query_host, INTERNAL_IP.clone(), &INTRANET_HOST, &intranet_subdomains) {
                  Ok(mut ip) => {
                    //TODO: make sure question type is all records (255) or A (1) (unrelated, CNAME is 5),
                    //extract_type_bytes_from_dns_query?
                    //
                    //firefox blocks us, very sad
                    if ip[0] == 192 && ip[1] == 168 {
                      ip = utils::ip_string_to_u8_array(&INTERNAL_IP);
                    }
                    //construct response
                    resp_start_bytes[5] = 1; //# of q: 1
                    //append question to resp_start_bytes (yes, this is a response, but question needs to be copied from query, apparently)
                    //label
                    resp_start_bytes.append(&mut query_hostname_to_label_bytes(&query_host));
                    //type and class are A (1) and IN (1)
                    resp_start_bytes.extend_from_slice(&[0 as u8, 1 as u8]);
                    resp_start_bytes.extend_from_slice(&[0 as u8, 1 as u8]);
                    //append answer to resp_start_bytes
                    //offset to the label in the front
                    resp_start_bytes.extend_from_slice(&[192 as u8, 12 as u8]);
                    //resp_start_bytes.append(&mut query_hostname_to_label_bytes(&query_host));
                    //type and class are A (1) and IN (1)
                    resp_start_bytes.extend_from_slice(&[0 as u8, 1 as u8]);
                    resp_start_bytes.extend_from_slice(&[0 as u8, 1 as u8]);
                    //TTL (arbitrarily pick 10 minutes, or 600 seconds [0x0258])
                    resp_start_bytes.extend_from_slice(&[0 as u8, 0 as u8, 2 as u8, 88 as u8]);
                    //RD LENGTH is two bytes, A record is 4 bytes
                    resp_start_bytes.push(0);
                    resp_start_bytes.push(4);
                    //RDDATA
                    resp_start_bytes.extend_from_slice(&ip);
                    let response = Response::from_data(resp_start_bytes).with_status_code(200).with_header(Header::from_str("Content-Type: application/dns-message").unwrap()).with_header(Header::from_str("Accept: application/dns-message").unwrap());
                    let _ = request.respond(response);
                  },
                  Err(QueryError::NXDomain) => {
                    //Not found
                    //explanation of below, see above where resp_start_bytes is defined
                    resp_start_bytes[2] = 3;
                    resp_start_bytes[7] = 0;
                    //need to send 200 even if nxdomain, see rfc8484 4.2.1
                    let response = Response::from_data(resp_start_bytes).with_status_code(200).with_header(Header::from_str("Content-Type: application/dns-message").unwrap()).with_header(Header::from_str("Accept: application/dns-message").unwrap());
                    let _ = request.respond(response);
                  },
                  Err(QueryError::NonIntranet) => {
                    //regular domain, ens or handshake domain
                    //hnsdns handles all, how nice. No adblock though, like mullvad...
                    //forward query to https://doh.hnsdns.com/dns-query, and return what it returns
                    let mut header_map = HeaderMap::new();
                    header_map.insert(ACCEPT, "application/dns-message".parse().unwrap());
                    header_map.insert(CONTENT_TYPE, "application/dns-message".parse().unwrap());
                    let try_res = client.post(&NON_INTRANET_DOH).body(dns_query).headers(header_map).send(); //in the future, throw 500 if fails
                                                                                                     if let Ok(res) = try_res {
                      let res_status = res.status().as_u16(); //todo: status should be 200
                      //println!("response from hnsdns: {:?}", res.bytes().unwrap().to_vec());
                      let response = Response::from_data(res.bytes().unwrap()).with_status_code(res_status).with_header(Header::from_str("Content-Type: application/dns-message").unwrap()).with_header(Header::from_str("Accept: application/dns-message").unwrap());
                      let _ = request.respond(response);
                    } else {
                      println!("SERVFAIL");
                      //should it be a SERVFAIL dns reply?
                      resp_start_bytes[2] = 2;
                      resp_start_bytes[7] = 0;
                      //need to send 200 even if nxdomain, see rfc8484 4.2.1
                      let response = Response::from_data(resp_start_bytes).with_status_code(200).with_header(Header::from_str("Content-Type: application/dns-message").unwrap()).with_header(Header::from_str("Accept: application/dns-message").unwrap());
                      let _ = request.respond(response);
                    }
                  },
                }
              } else {
                //400 bad request, since could not find host in question section of query
                let _ = request.respond(Response::empty(400));
              }
            } else {
              //400 bad request, since missing dns post data or query,
              //or wrong content-type on post
              let _ = request.respond(Response::empty(400));
            }
          } else if method == &Method::Options {
            //
            let response = Response::empty(200).with_header(Header::from_str("Content-Type: application/dns-message").unwrap()).with_header(Header::from_str("Accept: application/dns-message").unwrap());
            //.with_header(Header::from_str("Access-Control-Allow-Origin: *").unwrap()).with_header(Header::from_str("Access-Control-Allow-Headers: *").unwrap());
            let _ = request.respond(response);
          } else {
            //405 method not allowed
            let _ = request.respond(Response::empty(405));
          }
        } else {
          //404 not found
          let _ = request.respond(Response::empty(404));
        }
      } else if host.ends_with(INTRANET_HOST) {
        let intranet_subdomain = extract_subdomain(&host, INTRANET_HOST);
        let maybe_subdomain_info = intranet_subdomains.get(intranet_subdomain);
        if let Some(subdomain_info) = maybe_subdomain_info {
          //though we have https cert, we need the other end to have https cert to for security
          let port_string = if let Some(port) = subdomain_info.port { ":".to_owned() + &port.to_string() } else { String::new() };
          //let request_url = "https://".to_owned() + &subdomain_info.ip_string + &port_string
          let protocol = if subdomain_info.proxy_use_http == true { "http://" } else { "https://" };
          let request_url = protocol.to_owned() + &subdomain_info.ip_string + &port_string + path;
          let mut proxy_req = self_cert_client.request(reqwest::Method::from_str(method.as_str()).unwrap(), request_url);//.timeout(std::time::Duration::MAX);
          //if body
          //headers
          for tiny_header in request.headers() {
            let header_name = tiny_header.field.as_str().as_str();
            let mut header_value = tiny_header.value.as_str().to_string();
            if header_name == "Range" && header_value.starts_with("bytes=") && header_value.ends_with("-") {
              header_value = utils::limit_open_ended_range(&header_value);
            }
            proxy_req = proxy_req.header(reqwest::header::HeaderName::from_str(header_name).unwrap(), reqwest::header::HeaderValue::from_str(&header_value).unwrap());
          }
          if request.body_length().is_some() {
            let mut body = Vec::new();
            request.as_reader().read_to_end(&mut body).unwrap();
            proxy_req = proxy_req.body(body);
          }
          let try_res = proxy_req.send();
          if let Ok(proxy_res) = try_res {
            let res_status = proxy_res.status().as_u16();
            let res_headers = proxy_res.headers().clone();
            let mut response = Response::from_data(proxy_res.bytes().unwrap()).with_status_code(res_status);
            //add headers to response
            for (field, value) in res_headers.iter() {
              response.add_header(Header::from_str(&format!("{}: {}", field, value.to_str().unwrap())).unwrap());
            }
            let _ = request.respond(response);
          }
          //
        } else {
          //421 misdirected, since we dont know wtf this subdomain is. it's not us!
          let _ = request.respond(Response::empty(421));
        }
      } else {
        //let intranet_subdomain = extract_subdomain(&host, INTRANET_HOST);
        if is_intranet_subdomain(&host, &intranet_subdomains) {
          //request to intranet tld, respond with redirect to intranet site
          println!("redirecting {} to elintra.net subdomain", &host);
          //301, moved permanently
          let _ = request.respond(Response::empty(301).with_header(Header::from_str(&("Location: https://".to_owned() + &host + "." + INTRANET_HOST)).unwrap()));
        } else {
          //421 misdirected, since we dont know wtf this host is. it's not us!
          let _ = request.respond(Response::empty(421));
        }
      }
    } else {
      //400 bad request, since missing Host header
      let _ = request.respond(Response::empty(400));
    }
  }
}
