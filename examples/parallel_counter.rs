fn main() { unimplemented!("TODO") }

/*
extern crate desync;
extern crate flo_stream;
extern crate futures;
extern crate rand;

use desync::*;
use flo_stream::*;
use futures::*;
use futures::stream;
use futures::executor;

use std::sync::*;
use std::time;

fn main() {
    // Demonstrates how the single publisher can be used with desync to schedule work across multiple threads

    // Task that takes some chunks of work (vectors of numbers) and counts the number of 0s in each, returning a stream of results
    fn count_zeros<In: 'static+Send+Stream<Item=Vec<u32>, Error=()>>(input: In) -> impl Stream<Item=u32, Error=()> {
        // There's no state, so we desync around a void type
        let worker = Arc::new(Desync::new(()));

        // Counts the number of 0s in the input vector, asynchronously
        pipe(worker, input, |_state, next| {
            let mut count = 0;

            // Do 10ms of actual 'work' (busy waiting)
            let mut _some_count = 0;
            let start = time::SystemTime::now();
            while time::SystemTime::now().duration_since(start).unwrap() < time::Duration::from_millis(10) {
                _some_count += 1;
            }
            
            if let Ok(next) = next {
                for val in next {
                    if val == 0 {
                        count += 1;
                    }
                }
            }

            Ok(count)
        })
    }

    // Buffer size of 1 means that this will generate back pressure when any worker is busy
    let mut work_publisher = SinglePublisher::new(1);

    // Create 5 workers to receive work from the publisher
    let workers = (0..5).into_iter()
        .map(|_| count_zeros(work_publisher.subscribe()))
        .collect::<Vec<_>>();

    // Input stream is 10,000,000 random numbers (in a release build you might want to try 100_000_000 or more)
    let input_stream = stream::iter_ok::<_, ()>((0..10_000_000)
        .into_iter()
        .map(|_| rand::random::<u32>() % 1024));
    
    // Slice into chunks with 32k numbers each
    let input_work = input_stream.chunks(32000);

    // Send the chunks to the work publisher to be scheduled
    let work_done = work_publisher.send_all(input_work);

    // Count up the results in another desync object
    let final_count = Arc::new(Desync::new(0));
    workers.into_iter().for_each(|worker| {
        pipe_in(final_count.clone(), worker, |state, next| {
            *state += next.unwrap_or(0);
            println!("So far: {}", *state);
        });
    });

    // Wait for the processing to finish
    executor::spawn(work_done).wait_future().unwrap();

    // Notify about the final count when we're done
    final_count.sync(|count| println!("Final count was {}", count));
}
*/